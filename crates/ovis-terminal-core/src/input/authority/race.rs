//! Classifying admitted writes after a credential change.

use ovis_terminal_holder::protocol::Input;
use ovis_terminal_protocol::domain::credential_rotation::{CommitRace, FenceToken, RacedWrite};
use ovis_terminal_protocol::schema::scalar::EventSeq;

use crate::input::receipts::{Delivery, Receipt};
use crate::input::sequencer::{Sequencer, SequencerError, View};

/// The fate of a write so far, or `None` while it waits in the queue and will be rechecked
/// again at its write.
pub fn race(receipt: &Receipt) -> Option<CommitRace> {
    match receipt.delivery {
        Delivery::Queued => None,
        Delivery::Fenced { reason } => Some(CommitRace::CancelledBeforeCommit {
            reason: FenceToken::new(reason.kind().to_owned()),
        }),
        Delivery::PtyWritten {
            written_bytes,
            output_offset,
        }
        | Delivery::Partial {
            written_bytes,
            output_offset,
        } => Some(CommitRace::AlreadyCommitted {
            written_bytes,
            output_offset,
        }),
        Delivery::OutcomeUnknown => Some(CommitRace::OutcomeUnknown),
    }
}

fn write(receipt: &Receipt, race: CommitRace) -> RacedWrite {
    RacedWrite::new(
        (
            receipt.op_id.clone(),
            EventSeq::new(receipt.queued_seq.get()),
        ),
        receipt.admitted_bytes,
        race,
    )
}

/// The wire record of a settled write; `None` while it is still queued.
pub fn raced(receipt: &Receipt) -> Option<RacedWrite> {
    race(receipt).map(|race| write(receipt, race))
}

/// What one recheck of a terminal's admitted writes settled.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct Recheck {
    /// Writes the recheck cancelled with zero bytes, and the write with the holder whose
    /// fence is unanswered, in admission order.
    pub raced: Vec<RacedWrite>,
    /// The empty holder write that fences the entry with the holder; hand it to the holder
    /// so a write it has not started is dropped.
    pub fence: Option<Input>,
}

/// Rechecks every admitted write of `sequencer` against `view` now.
///
/// # Errors
///
/// `limit_exceeded` when the fence epoch or holder write sequence is exhausted.
pub fn recheck(sequencer: &mut Sequencer, view: &View<'_>) -> Result<Recheck, SequencerError> {
    let step = sequencer.fence(view)?;
    let mut raced: Vec<RacedWrite> = step.fenced.iter().filter_map(raced).collect();
    if step.dispatch.is_some()
        && let Some(receipt) = sequencer.with_holder()
    {
        raced.push(write(receipt, CommitRace::OutcomeUnknown));
    }
    raced.sort_by_key(|write| write.queued_seq);
    Ok(Recheck {
        raced,
        fence: step.dispatch,
    })
}
