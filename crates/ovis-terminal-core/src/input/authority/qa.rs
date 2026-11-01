//! Rechecking admitted writes when a credential is withdrawn: every write is reported as
//! cancelled before commit, already committed, or of unknown outcome; none is resent.

use ovis_terminal_core::input::authority::{race, recheck};
use ovis_terminal_core::input::sequencer::{Payload, QueueLimits};
use ovis_terminal_protocol::domain::credential_rotation::{CommitRace, FenceToken};
use ovis_terminal_protocol::schema::methods::input::Submit;
use ovis_terminal_protocol::schema::scalar::{ByteCount, ByteOffset};

use super::TestResult;
use super::fixture::{Fixture, WRITER, ack, admit, modes, submission, view};

const fn text(text: &str) -> Payload<'_> {
    Payload::Text {
        text,
        submit: &Submit::None,
    }
}

fn cancelled(reason: &str) -> CommitRace {
    CommitRace::CancelledBeforeCommit {
        reason: FenceToken::new(reason.to_owned()),
    }
}

#[test]
fn withdrawal_reports_cancelled_unknown_then_committed_and_never_resends() -> TestResult {
    // Given three writes of one writer, the first handed to the holder and unacknowledged.
    let fixture = Fixture::new()?;
    let (leases, modes) = (fixture.leases()?, modes(b"")?);
    let writer = fixture.client("writer", WRITER)?;
    let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;
    for op in ["one", "two", "six"] {
        admit(
            &mut sequencer,
            &view(&leases, &modes),
            (
                writer.writer(),
                submission(op, fixture.expected()?, text(op)),
            ),
        )?;
    }
    let first = sequencer
        .next(&view(&leases, &modes))?
        .dispatch
        .ok_or("no write")?;

    // When its credential is withdrawn and the runtime rechecks at once.
    fixture.revoke(&writer)?;
    let swept = recheck(&mut sequencer, &view(&leases, &modes))?;

    // Then the queued writes are cancelled with zero bytes, the one with the holder is of
    // unknown outcome until the holder answers, and a fence write is handed back.
    let outcomes: Vec<_> = swept
        .raced
        .iter()
        .map(|write| (write.op_id.as_str().to_owned(), write.race.clone()))
        .collect();
    assert_eq!(
        outcomes,
        [
            ("one".to_owned(), CommitRace::OutcomeUnknown),
            ("two".to_owned(), cancelled("credential_revoked")),
            ("six".to_owned(), cancelled("credential_revoked")),
        ]
    );
    let fence = swept
        .fence
        .ok_or("no fence write for the entry with the holder")?;
    assert_eq!(fence.bytes, b"");
    assert!(fence.grant.fence_epoch > first.grant.fence_epoch);

    // And when the holder reports it had already written the first entry in full, that
    // write is already committed with its delivery facts, and nothing else is ever sent.
    sequencer.acknowledge(&ack(&fence, 0))?;
    let settled = sequencer
        .acknowledge(&ack(&first, 3))?
        .ok_or("the acknowledgement settled nothing")?;
    assert_eq!(
        race(&settled),
        Some(CommitRace::AlreadyCommitted {
            written_bytes: ByteCount::new(3),
            output_offset: ByteOffset::new(0),
        })
    );
    assert!(sequencer.next(&view(&leases, &modes))?.dispatch.is_none());
    assert_eq!(recheck(&mut sequencer, &view(&leases, &modes))?.raced, []);
    Ok(())
}

#[test]
fn withdrawing_one_writer_leaves_another_writers_queue_and_order_intact() -> TestResult {
    // Given two writers interleaved in one queue, nothing with the holder yet.
    let fixture = Fixture::new()?;
    let (leases, modes) = (fixture.leases()?, modes(b"")?);
    let (gone, kept) = (
        fixture.client("gone", WRITER)?,
        fixture.client("kept", WRITER)?,
    );
    let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;
    for (client, op) in [(&gone, "g1"), (&kept, "k1"), (&gone, "g2"), (&kept, "k2")] {
        admit(
            &mut sequencer,
            &view(&leases, &modes),
            (
                client.writer(),
                submission(op, fixture.expected()?, text(op)),
            ),
        )?;
    }

    // When one writer's credential is withdrawn.
    fixture.revoke(&gone)?;
    let swept = recheck(&mut sequencer, &view(&leases, &modes))?;

    // Then only its writes are cancelled, no fence write is needed, and the other
    // writer's writes go out next in admission order.
    let cancelled_ops: Vec<_> = swept
        .raced
        .iter()
        .map(|write| write.op_id.as_str())
        .collect();
    assert_eq!(cancelled_ops, ["g1", "g2"]);
    assert!(swept.fence.is_none());
    let next = sequencer
        .next(&view(&leases, &modes))?
        .dispatch
        .ok_or("no write")?;
    assert_eq!(next.bytes, b"k1");
    let still_queued = sequencer
        .receipt(&ovis_terminal_protocol::errors::OpId::new("k2".to_owned()))
        .ok_or("no receipt")?;
    assert_eq!(race(still_queued), None);
    Ok(())
}
