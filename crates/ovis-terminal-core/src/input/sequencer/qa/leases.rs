//! Lease follow-ups: a released lease never writes again and geometry outlives interact.

use ovis_terminal_core::input::receipts::{Delivery, FenceReason};
use ovis_terminal_core::input::sequencer::{Payload, QueueLimits};
use ovis_terminal_core::leases::{Acquire, ControlSlot, LeaseSlot};
use ovis_terminal_protocol::schema::methods::input::Submit;

use super::TestResult;
use super::fixture::{Fixture, WRITER, admit, modes, submission, view};

#[test]
fn the_same_owner_reacquiring_cannot_write_through_its_released_lease() -> TestResult {
    // Given an entry queued under lease 1 that its owner then released and reacquired.
    let mut fixture = Fixture::new()?;
    let (mut leases, modes) = (fixture.leases()?, modes(b"")?);
    let operator = fixture.client("operator", WRITER)?;
    let first = fixture.interact(&mut leases, &operator)?;
    let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;
    let payload = Payload::Text {
        text: "old",
        submit: &Submit::Enter,
    };
    admit(
        &mut sequencer,
        &view(&leases, &modes),
        (
            operator.writer().presenting(first),
            submission("old", fixture.expected()?, payload),
        ),
    )?;
    leases.release(operator.caller(), first)?;
    let second = fixture.interact(&mut leases, &operator)?;

    // When the write boundary is reached.
    let step = sequencer.next(&view(&leases, &modes))?;
    let stale = admit(
        &mut sequencer,
        &view(&leases, &modes),
        (
            operator.writer().presenting(first),
            submission("again", fixture.expected()?, payload),
        ),
    );

    // Then the old entry is fenced, the old lease admits nothing, and the new one writes.
    let fenced: Vec<_> = step
        .fenced
        .iter()
        .map(|receipt| (receipt.queued_seq.get(), receipt.delivery))
        .collect();
    assert_eq!(
        fenced,
        [(
            1,
            Delivery::Fenced {
                reason: FenceReason::LeaseRevoked
            }
        )]
    );
    assert!(step.dispatch.is_none());
    assert_eq!(stale.map_err(|error| error.kind()).err(), Some("no_lease"));
    admit(
        &mut sequencer,
        &view(&leases, &modes),
        (
            operator.writer().presenting(second),
            submission("new", fixture.expected()?, payload),
        ),
    )?;
    assert!(sequencer.next(&view(&leases, &modes))?.dispatch.is_some());
    Ok(())
}

#[test]
fn releasing_interact_keeps_geometry_and_still_requires_a_lease_to_write() -> TestResult {
    // Given an operator owning interact and geometry.
    let mut fixture = Fixture::new()?;
    let (mut leases, modes) = (fixture.leases()?, modes(b"")?);
    let operator = fixture.client("operator", WRITER)?;
    let interact = fixture.interact(&mut leases, &operator)?;
    let geometry = fixture.fresh()?;
    leases.acquire(
        operator.caller(),
        Acquire::new(ControlSlot::Geometry, geometry),
    )?;
    let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;

    // When only the interact lease is released.
    leases.release(operator.caller(), interact)?;
    let state = leases.view();

    // Then geometry stays with the operator and no write is admitted without interact.
    assert!(state.interact.is_none());
    assert_eq!(
        state.geometry.map(|lease| (lease.lease_id, lease.slot)),
        Some((geometry, LeaseSlot::Geometry))
    );
    let payload = Payload::Text {
        text: "x",
        submit: &Submit::None,
    };
    for writer in [
        operator.writer(),
        operator.writer().presenting(interact),
        operator.writer().presenting(geometry),
    ] {
        let refused = admit(
            &mut sequencer,
            &view(&leases, &modes),
            (writer, submission("x", fixture.expected()?, payload)),
        );
        assert_eq!(
            refused.map_err(|error| error.kind()).err(),
            Some("no_lease")
        );
    }
    Ok(())
}
