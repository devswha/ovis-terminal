//! A lease belongs to the connection that acquired it: another connection of the same
//! principal under the same client label competes like any other owner.

use ovis_terminal_core::leases::{
    Acquire, ControlSlot, LeaseError, LeaseEvent, RevokeReason, Write, WriteKind,
};

use super::TestResult;
use super::fixture::{Fixture, TAKEOVER, takeover};

#[test]
fn a_second_connection_under_the_holders_label_is_refused_and_cannot_borrow_its_lease() -> TestResult
{
    // Given one principal connected twice as "sdk", the first connection holding interact.
    let mut fixture = Fixture::new()?;
    let (first, second) = fixture.twins("sdk", TAKEOVER)?;
    let mut leases = fixture.leases()?;
    let held = leases.acquire(
        first.caller(),
        Acquire::new(ControlSlot::Interact, fixture.fresh()?),
    )?;

    // When the second connection acquires interact, then writes with the holder's lease id.
    let competing = leases
        .acquire(
            second.caller(),
            Acquire::new(ControlSlot::Interact, fixture.fresh()?),
        )
        .err();
    let borrowed = leases
        .authorize_write(
            second.caller(),
            Write::new(WriteKind::Input, Some(held.lease.lease_id)),
        )
        .err();

    // Then both are refused naming the holder, and only the holder still writes and
    // re-acquires its own lease without a transition.
    assert!(matches!(&competing, Some(LeaseError::Conflict { holder }) if **holder == held.lease));
    assert_eq!(competing.map(|error| error.kind()), Some("lease_conflict"));
    assert_eq!(borrowed.map(|error| error.kind()), Some("no_lease"));
    assert_eq!(leases.view().interact, Some(held.lease.clone()));
    let write = leases.authorize_write(
        first.caller(),
        Write::new(WriteKind::Input, Some(held.lease.lease_id)),
    )?;
    assert_eq!(write.lease(), Some(&held.lease));
    let again = leases.acquire(
        first.caller(),
        Acquire::new(ControlSlot::Interact, fixture.fresh()?),
    )?;
    assert_eq!(again.lease, held.lease);
    assert_eq!(again.transition.events, []);
    Ok(())
}

#[test]
fn disconnecting_one_connection_never_revokes_the_other_connections_leases() -> TestResult {
    // Given the first connection holds interact and the second, under the same label, observes.
    let mut fixture = Fixture::new()?;
    let (first, second) = fixture.twins("sdk", TAKEOVER)?;
    let mut leases = fixture.leases()?;
    let held = leases.acquire(
        first.caller(),
        Acquire::new(ControlSlot::Interact, fixture.fresh()?),
    )?;
    let watch = leases.observe(second.caller(), fixture.fresh()?)?;

    // When the second connection drops.
    let dropped = leases.disconnect(&second.owner())?;

    // Then exactly its own observe lease is revoked and the holder keeps interact.
    assert!(matches!(
        dropped.events.as_slice(),
        [LeaseEvent::Revoked { lease, reason: RevokeReason::Disconnected, .. }]
            if *lease == watch.lease
    ));
    let view = leases.view();
    assert_eq!(view.interact, Some(held.lease));
    assert_eq!(view.observers, []);
    Ok(())
}

#[test]
fn only_an_explicit_takeover_at_the_current_epoch_moves_the_lease_between_connections() -> TestResult
{
    // Given the first connection of "sdk" holds interact.
    let mut fixture = Fixture::new()?;
    let (first, second) = fixture.twins("sdk", TAKEOVER)?;
    let mut leases = fixture.leases()?;
    let held = leases.acquire(
        first.caller(),
        Acquire::new(ControlSlot::Interact, fixture.fresh()?),
    )?;

    // When the second connection takes over at the current epoch, then repeats it.
    let request = takeover(ControlSlot::Interact, held.lease.epoch, fixture.fresh()?)?;
    let taken = leases.take_over(second.caller(), &request)?;
    let repeated = leases.take_over(second.caller(), &request).err();
    let stale = leases
        .authorize_write(
            first.caller(),
            Write::new(WriteKind::Input, Some(held.lease.lease_id)),
        )
        .err();

    // Then the lease moved to the second connection, which cannot take its own lease, and
    // the first connection's old lease no longer writes.
    assert_eq!(taken.lease.owner, second.owner());
    assert_ne!(taken.lease.owner, first.owner());
    assert_eq!(repeated.map(|error| error.kind()), Some("self_lease"));
    assert_eq!(stale.map(|error| error.kind()), Some("no_lease"));
    Ok(())
}
