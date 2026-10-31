//! Geometry ownership: independent of interact and handed on only in acquisition order.

use ovis_terminal_core::leases::{Acquire, ControlSlot, LeaseEvent, RevokeReason};

use super::TestResult;
use super::fixture::{Fixture, INTERACTOR, OBSERVER, TAKEOVER, takeover};

#[test]
fn geometry_passes_in_acquisition_order_never_to_the_most_active_client() -> TestResult {
    // Given the operator owns geometry and two browsers queued for it, first then second.
    let mut fixture = Fixture::new()?;
    let operator = fixture.client("operator", INTERACTOR)?;
    let first = fixture.client("first", INTERACTOR)?;
    let second = fixture.client("second", INTERACTOR)?;
    let mut leases = fixture.leases()?;
    let input = leases.acquire(
        operator.caller(),
        Acquire::new(ControlSlot::Interact, fixture.fresh()?),
    )?;
    let grid = leases.acquire(
        operator.caller(),
        Acquire::new(ControlSlot::Geometry, fixture.fresh()?),
    )?;
    let first_claim = fixture.fresh()?;
    let queued = leases.acquire(
        first.caller(),
        Acquire::new(ControlSlot::Geometry, first_claim),
    );
    let second_claim = fixture.fresh()?;
    leases
        .acquire(
            second.caller(),
            Acquire::new(ControlSlot::Geometry, second_claim),
        )
        .err();
    for _ in 0..3 {
        let busy = Acquire::new(ControlSlot::Geometry, fixture.fresh()?);
        leases.acquire(second.caller(), busy).err();
        leases.observe(second.caller(), fixture.fresh()?)?;
    }

    // When the operator releases geometry.
    let released = leases.release(operator.caller(), grid.lease.lease_id)?;

    // Then the first requester receives it under the id it claimed, at the next epoch.
    assert_eq!(
        queued.err().map(|error| error.kind()),
        Some("lease_conflict")
    );
    let [
        LeaseEvent::Revoked {
            lease: gone,
            reason: RevokeReason::Released,
            displaced_by: None,
        },
        LeaseEvent::Geometry {
            lease: moved,
            previous: Some(previous),
        },
    ] = released.events.as_slice()
    else {
        return Err(format!("unexpected release events: {:?}", released.events).into());
    };
    assert_eq!((gone, previous), (&grid.lease, &grid.lease));
    assert_eq!(moved.owner, first.owner());
    assert_eq!(moved.lease_id, first_claim);
    assert_eq!(moved.epoch, grid.lease.epoch.checked_next()?);
    assert_eq!(leases.view().geometry_queue, vec![second.owner()]);

    // And a disconnect hands it to the next requester, then a final release leaves no owner.
    let dropped = leases.disconnect(&first.owner())?;
    assert!(matches!(
        dropped.events.as_slice(),
        [LeaseEvent::Revoked { reason: RevokeReason::Disconnected, .. }, LeaseEvent::Geometry { lease, .. }]
            if lease.owner == second.owner() && lease.lease_id == second_claim
    ));
    let owner = leases.view().geometry.ok_or("geometry not transferred")?;
    let last = leases.release(second.caller(), owner.lease_id)?;
    assert!(matches!(
        last.events.as_slice(),
        [LeaseEvent::Revoked { .. }]
    ));
    assert!(leases.view().geometry.is_none());
    assert_eq!(leases.view().interact, Some(input.lease));
    Ok(())
}

#[test]
fn interact_and_geometry_epochs_move_independently() -> TestResult {
    // Given the operator holds both slots.
    let mut fixture = Fixture::new()?;
    let operator = fixture.client("operator", INTERACTOR)?;
    let bridge = fixture.client("bridge", TAKEOVER)?;
    let mut leases = fixture.leases()?;
    let input = leases.acquire(
        operator.caller(),
        Acquire::new(ControlSlot::Interact, fixture.fresh()?),
    )?;
    let grid = leases.acquire(
        operator.caller(),
        Acquire::new(ControlSlot::Geometry, fixture.fresh()?),
    )?;

    // When the bridge takes over geometry only.
    let request = takeover(ControlSlot::Geometry, grid.lease.epoch, fixture.fresh()?)?;
    let taken = leases.take_over(bridge.caller(), &request)?;

    // Then geometry moved to the next epoch with both owners named, interact did not move.
    assert!(matches!(
        taken.transition.events.as_slice(),
        [
            LeaseEvent::Revoked { lease, reason: RevokeReason::TakenOver, displaced_by: Some(by) },
            LeaseEvent::Geometry { lease: now, previous: Some(old) },
        ] if lease == &grid.lease && by == &taken.lease && now == &taken.lease && old == &grid.lease
    ));
    assert_eq!(taken.lease.epoch, grid.lease.epoch.checked_next()?);
    assert_ne!(taken.lease.lease_id, input.lease.lease_id);
    assert_eq!(leases.view().interact, Some(input.lease));
    Ok(())
}

#[test]
fn an_observer_cannot_queue_for_geometry() -> TestResult {
    // Given the operator owns geometry.
    let mut fixture = Fixture::new()?;
    let operator = fixture.client("operator", INTERACTOR)?;
    let phone = fixture.client("phone", OBSERVER)?;
    let mut leases = fixture.leases()?;
    leases.acquire(
        operator.caller(),
        Acquire::new(ControlSlot::Geometry, fixture.fresh()?),
    )?;

    // When an observe-only principal asks for it.
    let refused = leases
        .acquire(
            phone.caller(),
            Acquire::new(ControlSlot::Geometry, fixture.fresh()?),
        )
        .err();

    // Then it is forbidden and never becomes a transfer candidate.
    assert_eq!(refused.map(|error| error.kind()), Some("forbidden"));
    assert_eq!(leases.view().geometry_queue, []);
    Ok(())
}
