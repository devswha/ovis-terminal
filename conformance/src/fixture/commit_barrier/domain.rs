//! The probe's record type and its 64 MiB snapshot producer.

use ovis_terminal_core::events::{EventData, EventError, Filter, SnapshotProducer};
use ovis_terminal_core::subscriptions::LossRecord;
use ovis_terminal_store::{EventPage, Revision, Snapshot};
use serde::{Deserialize, Serialize};

/// Bytes of terminal-history stand-in every snapshot carries beside its records.
pub(super) const HISTORY_BYTES: usize = 64 << 20;

/// One committed record, naming its source and position in that source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Barrier {
    Wrote { source: String, n: u64 },
    Lost(LossRecord),
}

impl From<LossRecord> for Barrier {
    fn from(loss: LossRecord) -> Self {
        Self::Lost(loss)
    }
}

impl EventData for Barrier {
    fn event_type(&self) -> &'static str {
        match self {
            Self::Wrote { .. } => "barrier.wrote",
            Self::Lost(_) => "subscription.lost",
        }
    }
}

/// The sequences visible at the snapshot's revision, read through its pinned view.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Captured {
    pub(super) seqs: Vec<u64>,
    pub(super) history: String,
}

#[derive(Debug)]
pub(super) struct Ballast;

impl SnapshotProducer for Ballast {
    type Sections = Captured;

    fn capture(&self, snapshot: &Snapshot, _filter: &Filter) -> Result<Captured, EventError> {
        let seqs = match snapshot.events_after(Revision::ZERO, 10_000)? {
            EventPage::Events(events) => events.iter().map(|event| event.seq.get()).collect(),
            _ => return Err(EventError::SnapshotUnavailable),
        };
        Ok(Captured {
            seqs,
            history: "h".repeat(HISTORY_BYTES),
        })
    }
}
