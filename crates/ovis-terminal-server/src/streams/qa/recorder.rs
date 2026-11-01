//! The stream scenarios' lease journal: commits through the real coordinator, then
//! publishes what it committed so a test can wait for a revocation.

use std::sync::Arc;
use std::time::Duration;

use ovis_terminal_core::events::{Coordinator, EventError, Filter, SnapshotProducer};
use ovis_terminal_core::leases::{LeaseEvent, Transition};
use ovis_terminal_server::streams::{LeaseJournal, StreamError};
use ovis_terminal_store::{OpId, Snapshot};
use tokio::sync::watch;

use super::TestResult;

const WAIT: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub(crate) struct NoSections;

impl SnapshotProducer for NoSections {
    type Sections = ();

    fn capture(&self, _snapshot: &Snapshot, _filter: &Filter) -> Result<(), EventError> {
        Ok(())
    }
}

pub(crate) type Core = Coordinator<LeaseEvent, NoSections>;

/// Commits through the real coordinator, then publishes what it committed.
#[derive(Debug)]
pub(crate) struct Recorder {
    core: Arc<Core>,
    committed: watch::Sender<Vec<LeaseEvent>>,
}

impl LeaseJournal for Recorder {
    fn record(&self, op: OpId, transition: Transition) -> Result<(), StreamError> {
        self.core.record(op, transition.clone())?;
        self.committed
            .send_modify(|events| events.extend(transition.events));
        Ok(())
    }
}

impl Recorder {
    /// A journal committing through `core` with nothing committed yet.
    pub(crate) fn new(core: Arc<Core>) -> Self {
        Self {
            core,
            committed: watch::channel(Vec::new()).0,
        }
    }

    /// Waits for a committed revocation of a lease held by client `label`.
    pub(crate) async fn revoked(&self, label: &str) -> TestResult {
        let mut committed = self.committed.subscribe();
        let wait = committed.wait_for(|events| {
            events.iter().any(|event| match event {
                LeaseEvent::Revoked { lease, .. } => lease.owner.client.as_str() == label,
                _ => false,
            })
        });
        tokio::time::timeout(WAIT, wait).await??;
        Ok(())
    }
}
