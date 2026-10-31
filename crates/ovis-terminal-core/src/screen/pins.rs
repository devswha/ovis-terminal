//! Immutable snapshots pinned by revision, evicted least recently used.

use std::collections::VecDeque;
use std::sync::Arc;

use ovis_terminal_vt::ContentRevision;

use super::frozen::Frozen;

#[derive(Debug)]
pub(super) struct Pins {
    snapshots: VecDeque<Arc<Frozen>>,
    capacity: usize,
}

impl Pins {
    pub(super) const fn new(capacity: usize) -> Self {
        Self {
            snapshots: VecDeque::new(),
            capacity,
        }
    }

    pub(super) fn find(&mut self, revision: ContentRevision) -> Option<Arc<Frozen>> {
        let index = self
            .snapshots
            .iter()
            .position(|snapshot| snapshot.revision() == revision)?;
        let snapshot = self.snapshots.remove(index)?;
        self.snapshots.push_back(Arc::clone(&snapshot));
        Some(snapshot)
    }

    pub(super) fn insert(&mut self, snapshot: Arc<Frozen>) {
        if self.snapshots.len() >= self.capacity {
            self.snapshots.pop_front();
        }
        self.snapshots.push_back(snapshot);
    }

    pub(super) fn forget(&mut self, revision: ContentRevision) {
        self.snapshots
            .retain(|snapshot| snapshot.revision() != revision);
    }
}
