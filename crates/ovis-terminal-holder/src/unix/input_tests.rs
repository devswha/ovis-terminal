//! Input queue admission against a reference model of both bounds and fences.

use proptest::prelude::*;

use super::config::QueueLimits;
use super::input::{InputQueue, PushRejected, QueueUsage, QueuedInput};
use crate::protocol::{ProcessIncarnation, WriteGrant};

const LIMITS: QueueLimits = QueueLimits::new(128, 8);

fn offered(sequence: u64, (length, fence_epoch): (usize, u64)) -> QueuedInput {
    let process_incarnation = ProcessIncarnation::new(1);
    let grant = WriteGrant {
        grant_epoch: 1,
        fence_epoch,
        process_incarnation,
    };
    QueuedInput {
        sequence,
        grant,
        bytes: vec![0; length],
    }
}

#[test]
fn empty_requests_are_bounded_by_the_entry_limit() {
    // Given: a queue whose byte budget is already full.
    let queue = InputQueue::new(QueueLimits::new(1, 1024));
    assert!(queue.push(offered(1, (1, 1))).outcome.is_ok());

    // When: 100,000 empty requests arrive behind it.
    let accepted = (2..=100_001_u64)
        .filter(|sequence| queue.push(offered(*sequence, (0, 1))).outcome.is_ok())
        .count();

    // Then: only the entry bound's remainder is admitted.
    assert_eq!(accepted, 1023);
    let rejected = queue.push(offered(100_002, (0, 1)));
    let Err(PushRejected::Full { queued, input }) = rejected.outcome else {
        panic!("an empty request into a full queue is rejected");
    };
    assert_eq!(
        queued,
        QueueUsage {
            bytes: 1,
            entries: 1024
        }
    );
    assert_eq!(input.sequence, 100_002);
}

proptest! {
    #[test]
    fn admission_matches_a_byte_and_entry_bounded_fenced_model(
        offers in proptest::collection::vec((0_usize..64, 0_u64..4), 1..40),
    ) {
        let queue = InputQueue::new(LIMITS);
        let mut model: Vec<(u64, usize, u64)> = Vec::new();
        let mut fence = 0_u64;
        for (sequence, offer) in (1_u64..).zip(offers) {
            let admission = queue.push(offered(sequence, offer));
            let (length, epoch) = offer;
            fence = fence.max(epoch);
            let (kept, revoked): (Vec<_>, Vec<_>) =
                model.into_iter().partition(|entry| entry.2 >= fence);
            model = kept;
            let mut revoked_sequences: Vec<u64> = revoked.iter().map(|entry| entry.0).collect();
            let stale = epoch < fence;
            if stale {
                revoked_sequences.push(sequence);
            }
            let fenced: Vec<u64> = admission.fenced.iter().map(|entry| entry.sequence).collect();
            prop_assert_eq!(fenced, revoked_sequences);
            let usage = QueueUsage {
                bytes: model.iter().map(|entry| entry.1).sum(),
                entries: model.len(),
            };
            let fits = usage.bytes.saturating_add(length) <= LIMITS.bytes
                && usage.entries < LIMITS.entries;
            match admission.outcome {
                Ok(()) if stale => {}
                Ok(()) => {
                    prop_assert!(fits);
                    model.push((sequence, length, epoch));
                }
                Err(PushRejected::Full { queued, input }) => {
                    prop_assert!(!fits);
                    prop_assert_eq!((queued, input.sequence), (usage, sequence));
                }
                Err(PushRejected::Closed) => prop_assert!(false, "queue is open"),
            }
        }
    }
}
