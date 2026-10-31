//! Holder loss, exit, barriers, query replies and interruption at each barrier.

use ovis_terminal_core::input::receipts::{Delivery, FenceReason, InputSeq};
use ovis_terminal_core::input::sequencer::{BarrierState, Payload, QueueLimits, Sequencer};
use ovis_terminal_protocol::schema::methods::input::Submit;
use ovis_terminal_protocol::schema::methods::server::WriteSequence;

use super::TestResult;
use super::fixture::{Client, Fixture, WRITER, ack, admit, modes, process, submission, view};

fn queue(fixture: &Fixture, sequencer: &mut Sequencer, writer: &Client) -> TestResult {
    let (leases, modes) = (fixture.leases()?, modes(b"")?);
    for op in ["aa", "bbb"] {
        let payload = Payload::Text {
            text: op,
            submit: &Submit::None,
        };
        admit(
            sequencer,
            &view(&leases, &modes),
            (
                writer.writer(),
                submission(op, fixture.expected()?, payload),
            ),
        )?;
    }
    Ok(())
}

fn delivery(sequencer: &Sequencer, op: &str) -> Option<Delivery> {
    sequencer
        .receipt(&ovis_terminal_protocol::errors::OpId::new(op.to_owned()))
        .map(|receipt| receipt.delivery)
}

#[test]
fn holder_loss_never_resends_and_late_evidence_still_resolves() -> TestResult {
    // Given two entries, the first sent to the holder.
    let fixture = Fixture::new()?;
    let (leases, modes) = (fixture.leases()?, modes(b"")?);
    let view = view(&leases, &modes);
    let alice = fixture.client("alice", WRITER)?;
    let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;
    queue(&fixture, &mut sequencer, &alice)?;
    let sent = sequencer.next(&view)?.dispatch.ok_or("no write")?;

    // When the holder link is lost and re-adopted three times.
    let lost = sequencer.link_lost();
    for _ in 0..3 {
        sequencer.stream_ready(process(1)?, WriteSequence::new(0));
        // Then nothing is ever sent again.
        assert!(sequencer.next(&view)?.dispatch.is_none());
        sequencer.link_lost();
    }

    // And the sent entry is unknown while the unsent one is fenced with zero bytes.
    let lost: Vec<_> = lost.iter().map(|receipt| receipt.delivery).collect();
    assert_eq!(
        lost,
        [
            Delivery::OutcomeUnknown,
            Delivery::Fenced {
                reason: FenceReason::Recovering
            }
        ]
    );
    // And a late acknowledgement of a partial write reports the exact prefix.
    let resolved = sequencer
        .acknowledge(&ack(&sent, 1))?
        .ok_or("not resolved")?;
    assert!(
        matches!(resolved.delivery, Delivery::Partial { written_bytes, .. } if written_bytes.get() == 1)
    );
    let repeated = sequencer.acknowledge(&ack(&sent, 1));
    assert_eq!(
        repeated.map_err(|error| error.kind()).err(),
        Some("stale_seq")
    );
    Ok(())
}

#[test]
fn interruption_at_each_barrier_leaves_one_truthful_outcome() -> TestResult {
    // Given the same two entries interrupted at the queue, write and ack barriers.
    for barrier in ["queue", "write", "ack"] {
        let fixture = Fixture::new()?;
        let (leases, modes) = (fixture.leases()?, modes(b"")?);
        let view = view(&leases, &modes);
        let alice = fixture.client("alice", WRITER)?;
        let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;
        queue(&fixture, &mut sequencer, &alice)?;
        if barrier != "queue" {
            let sent = sequencer.next(&view)?.dispatch.ok_or("no write")?;
            if barrier == "ack" {
                sequencer.acknowledge(&ack(&sent, 2))?;
            }
        }

        // When the program exits and the stream later comes back.
        sequencer.exited();
        sequencer.stream_ready(process(1)?, WriteSequence::new(0));

        // Then the first entry has exactly the outcome its barrier allows and none is resent.
        let first = delivery(&sequencer, "aa").ok_or("no receipt")?;
        let expected_first = match barrier {
            "queue" => matches!(
                first,
                Delivery::Fenced {
                    reason: FenceReason::Finished
                }
            ),
            "write" => first == Delivery::OutcomeUnknown,
            _ => matches!(first, Delivery::PtyWritten { .. }),
        };
        assert!(expected_first, "{barrier}: {first:?}");
        assert_eq!(
            delivery(&sequencer, "bbb"),
            Some(Delivery::Fenced {
                reason: FenceReason::Finished
            })
        );
        assert!(sequencer.next(&view)?.dispatch.is_none(), "{barrier}");
    }
    Ok(())
}

#[test]
fn barriers_follow_queue_write_and_acknowledgement() -> TestResult {
    // Given two admitted entries of 2 and 3 bytes.
    let fixture = Fixture::new()?;
    let (leases, modes) = (fixture.leases()?, modes(b"")?);
    let view = view(&leases, &modes);
    let alice = fixture.client("alice", WRITER)?;
    let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;
    queue(&fixture, &mut sequencer, &alice)?;
    let pending = |bytes: u64| BarrierState::Pending {
        pending_bytes: ovis_terminal_protocol::schema::scalar::ByteCount::new(bytes),
    };

    // When each entry is sent and acknowledged, the second only in part.
    assert_eq!(sequencer.barrier(InputSeq::new(2))?, pending(5));
    let first = sequencer.next(&view)?.dispatch.ok_or("no write")?;
    assert_eq!(sequencer.barrier(InputSeq::new(1))?, pending(2));
    sequencer.acknowledge(&ack(&first, 2))?;
    assert_eq!(sequencer.barrier(InputSeq::new(1))?, BarrierState::Reached);
    assert_eq!(sequencer.barrier(InputSeq::new(2))?, pending(3));
    let second = sequencer.next(&view)?.dispatch.ok_or("no write")?;
    sequencer.acknowledge(&ack(&second, 1))?;

    // Then the barrier names the undelivered entry and refuses positions never admitted.
    assert_eq!(
        sequencer.barrier(InputSeq::new(2))?,
        BarrierState::Undelivered {
            first: InputSeq::new(2)
        }
    );
    let ahead = sequencer.barrier(InputSeq::new(3));
    assert_eq!(ahead.map_err(|error| error.kind()).err(), Some("seq_ahead"));
    Ok(())
}

#[test]
fn query_replies_are_not_held_behind_a_blocked_user_entry() -> TestResult {
    // Given a user entry with the holder that is not acknowledged and one waiting.
    let fixture = Fixture::new()?;
    let (leases, modes) = (fixture.leases()?, modes(b"")?);
    let view = view(&leases, &modes);
    let alice = fixture.client("alice", WRITER)?;
    let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;
    queue(&fixture, &mut sequencer, &alice)?;
    let blocked = sequencer.next(&view)?.dispatch.ok_or("no write")?;

    // When the terminal answers a cursor position query.
    sequencer.admit_reply(b"\x1b[1;1R".to_vec())?;
    let reply = sequencer.next(&view)?.dispatch.ok_or("reply held back")?;

    // Then the reply goes out at once, after the blocked write, and never becomes a receipt.
    assert_eq!(reply.bytes, b"\x1b[1;1R");
    assert!(reply.sequence > blocked.sequence);
    assert!(sequencer.acknowledge(&ack(&reply, 6))?.is_none());
    assert!(sequencer.next(&view)?.dispatch.is_none());
    let overflow = (0..65)
        .map(|_| sequencer.admit_reply(vec![b'x']))
        .find(Result::is_err);
    assert_eq!(
        overflow.and_then(Result::err).map(|error| error.kind()),
        Some("input_overflow")
    );
    Ok(())
}
