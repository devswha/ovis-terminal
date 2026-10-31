//! Ownership, idempotence, disconnect and write-authority behavior of the interact lease.

use ovis_terminal_core::leases::{
    Acquire, ControlSlot, LeaseEvent, LeaseSlot, RevokeReason, Write, WriteKind,
};

use super::TestResult;
use super::fixture::{Fixture, INTERACTOR, NOW, OBSERVER, TAKEOVER, takeover};

#[test]
fn repeated_acquire_and_takeover_change_nothing_the_second_time() -> TestResult {
    // Given the bridge already took interact over once.
    let mut fixture = Fixture::new()?;
    let operator = fixture.client("operator", INTERACTOR)?;
    let bridge = fixture.client("bridge", TAKEOVER)?;
    let mut leases = fixture.leases()?;
    let held = leases.acquire(
        operator.caller(),
        Acquire::new(ControlSlot::Interact, fixture.fresh()?),
    )?;
    let again = leases.acquire(
        operator.caller(),
        Acquire::new(ControlSlot::Interact, fixture.fresh()?),
    )?;
    let request = takeover(ControlSlot::Interact, held.lease.epoch, fixture.fresh()?)?;
    let taken = leases.take_over(bridge.caller(), &request)?;
    let after_first = leases.view();

    // When the same takeover is retried after an interruption.
    let retried = leases.take_over(bridge.caller(), &request).err();

    // Then the retry is self_lease and neither the epoch nor the owner moved again.
    assert_eq!(again.lease, held.lease);
    assert_eq!(again.transition.events, []);
    assert_eq!(retried.map(|error| error.kind()), Some("self_lease"));
    assert_eq!(leases.view(), after_first);
    assert_eq!(after_first.interact, Some(taken.lease));
    Ok(())
}

#[test]
fn disconnect_revokes_every_lease_of_that_owner_in_slot_order() -> TestResult {
    // Given the operator holds interact, geometry and an observe stream.
    let mut fixture = Fixture::new()?;
    let operator = fixture.client("operator", INTERACTOR)?;
    let rival = fixture.client("rival", INTERACTOR)?;
    let mut leases = fixture.leases()?;
    let input = leases.acquire(
        operator.caller(),
        Acquire::new(ControlSlot::Interact, fixture.fresh()?),
    )?;
    let grid = leases.acquire(
        operator.caller(),
        Acquire::new(ControlSlot::Geometry, fixture.fresh()?),
    )?;
    let watch = leases.observe(operator.caller(), fixture.fresh()?)?;

    // When its connection drops.
    let dropped = leases.disconnect(&operator.owner())?;

    // Then interact, geometry and observe are revoked in that order and the slots free up.
    let revoked: Vec<_> = dropped
        .events
        .iter()
        .map(|event| match event {
            LeaseEvent::Revoked {
                lease,
                reason: RevokeReason::Disconnected,
                displaced_by: None,
            } => Some(lease.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        revoked,
        vec![Some(input.lease), Some(grid.lease), Some(watch.lease)]
    );
    let view = leases.view();
    assert!(view.interact.is_none() && view.geometry.is_none() && view.observers.is_empty());
    let next = leases.acquire(
        rival.caller(),
        Acquire::new(ControlSlot::Interact, fixture.fresh()?),
    )?;
    assert_eq!(next.lease.owner, rival.owner());
    assert_eq!(leases.disconnect(&operator.owner())?.events, []);
    Ok(())
}

#[test]
fn release_needs_the_owning_principal_not_the_lease_address() -> TestResult {
    // Given the operator holds interact and the rival learned its lease id.
    let mut fixture = Fixture::new()?;
    let operator = fixture.client("operator", INTERACTOR)?;
    let rival = fixture.client("rival", INTERACTOR)?;
    let mut leases = fixture.leases()?;
    let input = leases.acquire(
        operator.caller(),
        Acquire::new(ControlSlot::Interact, fixture.fresh()?),
    )?;

    // When the rival and then the operator release it.
    let foreign = leases.release(rival.caller(), input.lease.lease_id).err();
    let released = leases.release(operator.caller(), input.lease.lease_id)?;

    // Then only the owner's release revokes it.
    assert_eq!(foreign.map(|error| error.kind()), Some("no_lease"));
    assert!(matches!(
        released.events.as_slice(),
        [LeaseEvent::Revoked { lease, reason: RevokeReason::Released, displaced_by: None }]
            if lease == &input.lease
    ));
    assert!(leases.view().interact.is_none());
    Ok(())
}

#[test]
fn lease_free_writes_need_input_scope_and_stop_once_any_lease_exists() -> TestResult {
    // Given a terminal with no lease, an agent grant and an observer-only grant.
    let mut fixture = Fixture::new()?;
    let agent = fixture.client("agent", INTERACTOR)?;
    let phone = fixture.client("phone", OBSERVER)?;
    let mut leases = fixture.leases()?;
    let queued = leases.authorize_write(agent.caller(), Write::new(WriteKind::Input, None))?;
    let observer_write = leases
        .authorize_write(phone.caller(), Write::new(WriteKind::Input, None))
        .err();
    let raw = leases
        .authorize_write(agent.caller(), Write::new(WriteKind::Raw, None))
        .err();

    // When an observer attaches, so a lease now exists.
    leases.observe(phone.caller(), fixture.fresh()?)?;

    // Then the queued lease-free write is fenced and new lease-free writes are refused.
    assert!(queued.lease().is_none());
    assert_eq!(observer_write.map(|error| error.kind()), Some("forbidden"));
    assert_eq!(raw.map(|error| error.kind()), Some("forbidden"));
    assert_eq!(
        leases.recheck(&queued, NOW).err().map(|error| error.kind()),
        Some("no_lease")
    );
    let refused = leases
        .authorize_write(agent.caller(), Write::new(WriteKind::Input, None))
        .err();
    assert_eq!(refused.map(|error| error.kind()), Some("no_lease"));
    Ok(())
}

#[test]
fn observe_leases_never_resize_or_write() -> TestResult {
    // Given an operator with geometry, an observing phone and an interact-capable rival.
    let mut fixture = Fixture::new()?;
    let operator = fixture.client("operator", INTERACTOR)?;
    let rival = fixture.client("rival", INTERACTOR)?;
    let phone = fixture.client("phone", OBSERVER)?;
    let mut leases = fixture.leases()?;
    let no_grid = leases
        .authorize_resize(rival.caller(), fixture.fresh()?)
        .err();
    let grid = leases.acquire(
        operator.caller(),
        Acquire::new(ControlSlot::Geometry, fixture.fresh()?),
    )?;
    let watch = leases.observe(phone.caller(), fixture.fresh()?)?;
    let rival_watch = leases.observe(rival.caller(), fixture.fresh()?)?;

    // When each presents a lease it does not own for geometry or input.
    let observer_resize = leases
        .authorize_resize(phone.caller(), watch.lease.lease_id)
        .err();
    let observer_write = leases
        .authorize_write(
            phone.caller(),
            Write::new(WriteKind::Input, Some(watch.lease.lease_id)),
        )
        .err();
    let observe_role = leases
        .authorize_resize(rival.caller(), rival_watch.lease.lease_id)
        .err();
    let foreign_resize = leases
        .authorize_resize(rival.caller(), grid.lease.lease_id)
        .err();
    let owner_resize = leases.authorize_resize(operator.caller(), grid.lease.lease_id)?;

    // Then only the geometry owner may resize; the observer gains nothing.
    let kinds = [
        no_grid,
        observer_resize,
        observer_write,
        observe_role,
        foreign_resize,
    ]
    .map(|error| error.map(|error| error.kind()));
    assert_eq!(
        kinds,
        [
            Some("no_lease"),
            Some("forbidden"),
            Some("forbidden"),
            Some("observe_forbidden_op"),
            Some("not_geometry_owner"),
        ]
    );
    assert_eq!(
        owner_resize.lease().map(|lease| lease.slot),
        Some(LeaseSlot::Geometry)
    );
    Ok(())
}
