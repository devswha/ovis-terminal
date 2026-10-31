//! The last write boundary: revocation, takeover, credential, process and mode changes.

use ovis_terminal_core::input::receipts::{Delivery, FenceReason, Receipt};
use ovis_terminal_core::input::sequencer::{Payload, QueueLimits};
use ovis_terminal_core::leases::{ControlSlot, Takeover};
use ovis_terminal_protocol::schema::methods::attach::TakeoverRequest;
use ovis_terminal_protocol::schema::methods::input::Submit;
use ovis_terminal_protocol::schema::methods::server::WriteSequence;
use ovis_terminal_protocol::schema::scalar::LeaseEpoch;

use super::TestResult;
use super::fixture::{Fixture, TAKEOVER, WRITER, ack, admit, modes, process, submission, view};

fn fenced(receipts: &[Receipt]) -> Vec<(u64, Delivery)> {
    receipts
        .iter()
        .map(|receipt| (receipt.queued_seq.get(), receipt.delivery))
        .collect()
}

const fn reason(reason: FenceReason) -> Delivery {
    Delivery::Fenced { reason }
}

#[test]
fn takeover_fences_unsent_entries_and_the_unwritten_holder_entry() -> TestResult {
    // Given an operator's three queued entries, the first already with the holder.
    let mut fixture = Fixture::new()?;
    let (mut leases, modes) = (fixture.leases()?, modes(b"")?);
    let operator = fixture.client("operator", WRITER)?;
    let bridge = fixture.client("bridge", TAKEOVER)?;
    let lease = fixture.interact(&mut leases, &operator)?;
    let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;
    for op in ["one", "two", "three"] {
        let payload = Payload::Text {
            text: op,
            submit: &Submit::Enter,
        };
        admit(
            &mut sequencer,
            &view(&leases, &modes),
            (
                operator.writer().presenting(lease),
                submission(op, fixture.expected()?, payload),
            ),
        )?;
    }
    let first = sequencer
        .next(&view(&leases, &modes))?
        .dispatch
        .ok_or("no write")?;

    // When the bridge takes over explicitly at the operator's epoch.
    let request: TakeoverRequest =
        serde_json::from_value(serde_json::json!({"lease_epoch": "1", "reason": "bridge"}))?;
    assert_eq!(request.lease_epoch, LeaseEpoch::new(1));
    let fresh = fixture.fresh()?;
    leases.take_over(
        bridge.caller(),
        &Takeover::new(ControlSlot::Interact, &request, fresh)?,
    )?;
    let step = sequencer.fence(&view(&leases, &modes))?;

    // Then unsent entries are fenced with zero bytes and the holder entry gets a fence write.
    assert_eq!(
        fenced(&step.fenced),
        [
            (2, reason(FenceReason::LeaseRevoked)),
            (3, reason(FenceReason::LeaseRevoked))
        ]
    );
    let handshake = step.dispatch.ok_or("no fence write")?;
    assert_eq!(handshake.bytes, Vec::<u8>::new());
    assert!(handshake.grant.fence_epoch > first.grant.fence_epoch);
    assert_eq!(
        handshake.grant.process_incarnation,
        first.grant.process_incarnation
    );
    // And the holder dropping the entry unwritten reports the revocation, never a write.
    let settled = sequencer
        .acknowledge(&ack(&first, 0))?
        .ok_or("no receipt")?;
    assert_eq!(settled.delivery, reason(FenceReason::LeaseRevoked));
    assert!(sequencer.acknowledge(&ack(&handshake, 0))?.is_none());
    // And the new holder writes under its own epoch.
    let payload = Payload::Text {
        text: "four",
        submit: &Submit::Enter,
    };
    admit(
        &mut sequencer,
        &view(&leases, &modes),
        (
            bridge.writer().presenting(fresh),
            submission("four", fixture.expected()?, payload),
        ),
    )?;
    let next = sequencer
        .next(&view(&leases, &modes))?
        .dispatch
        .ok_or("no write")?;
    assert_eq!(
        (next.bytes.as_slice(), next.grant.grant_epoch),
        (&b"four\r"[..], 2)
    );
    Ok(())
}

#[test]
fn a_fence_never_claims_to_retract_bytes_the_holder_already_wrote() -> TestResult {
    // Given an entry with the holder when its lease is released.
    let mut fixture = Fixture::new()?;
    let (mut leases, modes) = (fixture.leases()?, modes(b"")?);
    let operator = fixture.client("operator", WRITER)?;
    let lease = fixture.interact(&mut leases, &operator)?;
    let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;
    let payload = Payload::Text {
        text: "sent",
        submit: &Submit::Enter,
    };
    admit(
        &mut sequencer,
        &view(&leases, &modes),
        (
            operator.writer().presenting(lease),
            submission("sent", fixture.expected()?, payload),
        ),
    )?;
    let input = sequencer
        .next(&view(&leases, &modes))?
        .dispatch
        .ok_or("no write")?;
    leases.release(operator.caller(), lease)?;
    assert!(sequencer.fence(&view(&leases, &modes))?.dispatch.is_some());

    // When the holder reports it had already written the entry.
    let settled = sequencer
        .acknowledge(&ack(&input, 5))?
        .ok_or("no receipt")?;

    // Then the receipt reports the write.
    assert!(
        matches!(settled.delivery, Delivery::PtyWritten { written_bytes, .. } if written_bytes.get() == 5)
    );
    Ok(())
}

#[test]
fn revoked_credentials_and_replaced_processes_fence_queued_entries() -> TestResult {
    // Given two writers with queued entries on an unleased terminal.
    let fixture = Fixture::new()?;
    let (leases, modes) = (fixture.leases()?, modes(b"")?);
    let view = view(&leases, &modes);
    let alice = fixture.client("alice", WRITER)?;
    let bob = fixture.client("bob", WRITER)?;
    let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;
    let payload = Payload::Text {
        text: "x",
        submit: &Submit::None,
    };
    for (writer, op) in [(&alice, "a"), (&bob, "b"), (&alice, "c")] {
        admit(
            &mut sequencer,
            &view,
            (
                writer.writer(),
                submission(op, fixture.expected()?, payload),
            ),
        )?;
    }

    // When alice's credential is revoked before the write boundary.
    fixture.revoke(&alice)?;
    let step = sequencer.next(&view)?;

    // Then her entries are fenced and bob's is written in order.
    assert_eq!(
        fenced(&step.fenced),
        [(1, reason(FenceReason::CredentialRevoked))]
    );
    let written = step.dispatch.ok_or("no write")?;
    sequencer.acknowledge(&ack(&written, 1))?;
    assert_eq!(
        fenced(&sequencer.next(&view)?.fenced),
        [(3, reason(FenceReason::CredentialRevoked))]
    );

    // When the process is replaced while bob's next entry is queued.
    admit(
        &mut sequencer,
        &view,
        (bob.writer(), submission("d", fixture.expected()?, payload)),
    )?;
    let replaced = sequencer.stream_ready(process(2)?, WriteSequence::new(written.sequence));

    // Then it is stale and never written; the old process cannot be expected any more.
    assert_eq!(fenced(&replaced), [(4, reason(FenceReason::StaleTarget))]);
    assert!(sequencer.next(&view)?.dispatch.is_none());
    let stale = admit(
        &mut sequencer,
        &view,
        (
            bob.writer(),
            submission("e", fixture.expected()?.with_process(process(1)?), payload),
        ),
    );
    assert_eq!(
        stale.map_err(|error| error.kind()).err(),
        Some("process_replaced")
    );
    Ok(())
}

#[test]
fn bytes_encoded_at_an_older_mode_revision_are_fenced() -> TestResult {
    // Given an arrow key encoded while cursor keys were in normal mode.
    let fixture = Fixture::new()?;
    let (leases, before) = (fixture.leases()?, modes(b"")?);
    let alice = fixture.client("alice", WRITER)?;
    let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;
    let keys = [String::from("up")];
    admit(
        &mut sequencer,
        &view(&leases, &before),
        (
            alice.writer(),
            submission("up", fixture.expected()?, Payload::Keys(&keys)),
        ),
    )?;

    // When the program switched to application cursor keys before the write.
    let after = modes(b"\x1b[?1h")?;
    assert_ne!(after.revision(), before.revision());
    let step = sequencer.next(&view(&leases, &after))?;

    // Then the stale encoding is never written.
    assert_eq!(
        fenced(&step.fenced),
        [(1, reason(FenceReason::ModeMismatch))]
    );
    assert!(step.dispatch.is_none());
    Ok(())
}
