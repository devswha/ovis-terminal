//! Split-tree edits that keep every surviving node and pane identity.

use std::collections::BTreeSet;

use super::model::{Child, Tab};
use crate::events::{EventError, ResourceId};
use crate::registry::{PaneId, SplitId, TerminalId};

impl Tab {
    pub(super) fn for_each_slot(&mut self, mut visit: impl FnMut(&mut Child)) {
        if let Some(root) = self.root.as_mut() {
            visit(root);
        }
        for split in &mut self.splits {
            visit(&mut split.first);
            visit(&mut split.second);
        }
    }

    pub(super) fn replace(&mut self, from: Child, to: Child) -> bool {
        let mut found = false;
        self.for_each_slot(|slot| {
            if *slot == from {
                *slot = to;
                found = true;
            }
        });
        found
    }

    /// Removes a pane leaf; its parent split collapses into the sibling.
    ///
    /// Returns the removed parent split, if the pane was not the root.
    pub(super) fn detach(&mut self, pane: PaneId) -> Result<Option<SplitId>, EventError> {
        let leaf = Child::Pane(pane);
        self.panes.retain(|record| record.id != pane);
        if self.zoomed == Some(pane) {
            self.zoomed = None;
        }
        if self.root == Some(leaf) {
            self.root = None;
            return Ok(None);
        }
        let index = self
            .splits
            .iter()
            .position(|split| split.first == leaf || split.second == leaf)
            .ok_or(EventError::InvalidRecord)?;
        let parent = self.splits.remove(index);
        let sibling = if parent.first == leaf {
            parent.second
        } else {
            parent.first
        };
        if !self.replace(Child::Split(parent.id), sibling) {
            return Err(EventError::InvalidRecord);
        }
        Ok(Some(parent.id))
    }

    pub(super) fn contents(&self) -> Vec<ResourceId> {
        std::iter::once(ResourceId::Tab(self.id))
            .chain(self.panes.iter().map(|pane| ResourceId::Pane(pane.id)))
            .chain(self.splits.iter().map(|split| ResourceId::Split(split.id)))
            .collect()
    }

    pub(super) fn terminals(&self) -> impl Iterator<Item = TerminalId> + '_ {
        self.panes.iter().filter_map(|pane| pane.terminal)
    }

    /// Rejects a persisted tree whose nodes are unreachable, shared or cyclic.
    pub(super) fn validate(&self) -> Result<(), EventError> {
        let mut seen = BTreeSet::new();
        let mut pending: Vec<Child> = self.root.into_iter().collect();
        while let Some(child) = pending.pop() {
            if !seen.insert(child) {
                return Err(EventError::InvalidRecord);
            }
            if let Child::Split(id) = child {
                let split = self
                    .splits
                    .iter()
                    .find(|split| split.id == id)
                    .ok_or(EventError::InvalidRecord)?;
                pending.extend([split.first, split.second]);
            }
        }
        let expected = self
            .panes
            .iter()
            .map(|pane| Child::Pane(pane.id))
            .chain(self.splits.iter().map(|split| Child::Split(split.id)))
            .collect::<BTreeSet<_>>();
        let zoom_ok = self
            .zoomed
            .is_none_or(|pane| expected.contains(&Child::Pane(pane)));
        if seen != expected
            || expected.len() != self.panes.len().saturating_add(self.splits.len())
            || !zoom_ok
        {
            return Err(EventError::InvalidRecord);
        }
        Ok(())
    }
}
