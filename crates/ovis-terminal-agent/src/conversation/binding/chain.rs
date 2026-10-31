//! History records and their chains: every record names its parent, so a fork or rewind
//! leaves the abandoned records in place, addressable but outside the active chain.
//!
//! A transcript line is `{"id", "parent"?, "role", "blocks"?}`; a line without `parent`
//! continues the line before it. Only newline-terminated lines are records, so a line
//! the writer has not finished is never read.

use std::collections::BTreeMap;

use ovis_terminal_protocol::schema::ids::RecordId;
use serde::Deserialize;
use serde_json::Value;

use super::BindingError;
use super::row::Selection;

/// Most records one history holds.
const RECORD_LIMIT: usize = 100_000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Line {
    id: RecordId,
    #[serde(default)]
    parent: Option<RecordId>,
    role: String,
    #[serde(default)]
    blocks: Vec<Value>,
}

/// One record and the index of its parent.
#[derive(Debug, Clone)]
pub(super) struct Record {
    pub id: RecordId,
    pub parent: Option<usize>,
    pub role: String,
    pub blocks: Vec<Value>,
}

/// All records of one observation in file order; parents always precede children.
#[derive(Debug, Default)]
pub(super) struct History {
    pub records: Vec<Record>,
    index: BTreeMap<RecordId, usize>,
}

impl History {
    /// Parses the complete lines of `bytes`.
    pub(super) fn parse(bytes: &[u8]) -> Result<Self, BindingError> {
        let mut history = Self::default();
        let Some(complete) = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .and_then(|end| bytes.get(..end))
        else {
            return Ok(history);
        };
        for (number, text) in complete.split(|byte| *byte == b'\n').enumerate() {
            let line = number.saturating_add(1);
            let malformed = || BindingError::Malformed { line };
            let parsed: Line = serde_json::from_slice(text).map_err(|_| malformed())?;
            let parent = match &parsed.parent {
                Some(id) => Some(history.find(id).ok_or_else(malformed)?),
                None => history.records.len().checked_sub(1),
            };
            if history.index.contains_key(&parsed.id) || history.records.len() >= RECORD_LIMIT {
                return Err(malformed());
            }
            history
                .index
                .insert(parsed.id.clone(), history.records.len());
            history.records.push(Record {
                id: parsed.id,
                parent,
                role: parsed.role,
                blocks: parsed.blocks,
            });
        }
        Ok(history)
    }

    /// Index of record `id`.
    pub(super) fn find(&self, id: &RecordId) -> Option<usize> {
        self.index.get(id).copied()
    }

    /// The active head: the newest record appended after `selected` was chosen that
    /// continues it, or the newest record when nothing was selected.
    pub(super) fn active_head(&self, selected: Option<&Selection>) -> Option<usize> {
        let Some((start, seen)) = selected.and_then(|selection| {
            let seen = usize::try_from(selection.seen).ok()?;
            Some((self.find(&selection.head)?, seen))
        }) else {
            return self.records.len().checked_sub(1);
        };
        let mut continues = vec![false; self.records.len()];
        let mut head = start;
        for (index, record) in self.records.iter().enumerate().skip(start) {
            let inherited = index >= seen
                && record
                    .parent
                    .and_then(|parent| continues.get(parent).copied())
                    .unwrap_or(false);
            if index == start || inherited {
                if let Some(flag) = continues.get_mut(index) {
                    *flag = true;
                }
                head = index;
            }
        }
        Some(head)
    }

    /// Indices from the root to `head`.
    pub(super) fn chain(&self, head: usize) -> Vec<usize> {
        let mut chain = Vec::new();
        let mut next = Some(head);
        while let Some(index) = next {
            chain.push(index);
            next = self.records.get(index).and_then(|record| record.parent);
        }
        chain.reverse();
        chain
    }

    /// Leaf records outside `chain`, in file order.
    pub(super) fn off_branch(&self, chain: &[usize]) -> Vec<RecordId> {
        let mut has_child = vec![false; self.records.len()];
        for parent in self.records.iter().filter_map(|record| record.parent) {
            if let Some(flag) = has_child.get_mut(parent) {
                *flag = true;
            }
        }
        self.records
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                !has_child.get(*index).copied().unwrap_or(true) && !chain.contains(index)
            })
            .map(|(_, record)| record.id.clone())
            .collect()
    }
}
