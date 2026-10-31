//! Persisted prompt rows and the runtime's prompt index, read and written in commit units.

use ovis_terminal_core::events::EventError;
use ovis_terminal_core::registry::{RuntimeId, TerminalId};
use ovis_terminal_protocol::schema::ids::PromptId;
use ovis_terminal_protocol::schema::methods::status::PromptRecord;
use ovis_terminal_store::{CasWrite, Payload, ResourceKey, ResourceLookup, Snapshot, Version};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::PromptError;

/// Most open prompts one terminal keeps.
pub(super) const OPEN_LIMIT: usize = 16;
/// Most closed prompts one terminal keeps readable; older ones are deleted.
const CLOSED_LIMIT: usize = 64;

/// A value and the version its compare-and-swap write must expect.
#[derive(Debug)]
pub(super) struct Versioned<T> {
    pub value: T,
    version: Version,
    key: ResourceKey,
}

impl<T: Serialize> Versioned<T> {
    pub(super) fn write(&self) -> Result<CasWrite, PromptError> {
        let bytes = serde_json::to_vec(&self.value)?;
        Ok(CasWrite::new(
            self.key.clone(),
            self.version,
            Some(Payload::from(bytes)),
        ))
    }

    pub(super) fn delete(&self) -> CasWrite {
        CasWrite::new(self.key.clone(), self.version, None)
    }
}

fn read<T: DeserializeOwned>(
    snapshot: &Snapshot,
    key: &ResourceKey,
) -> Result<(Option<T>, Version), PromptError> {
    match snapshot.resource(key).map_err(EventError::from)? {
        ResourceLookup::Live { value, version, .. } => {
            Ok((Some(serde_json::from_slice(value.as_bytes())?), version))
        }
        ResourceLookup::Tombstone { version, .. } => Ok((None, version)),
        ResourceLookup::Absent => Ok((None, Version::ABSENT)),
        _ => Err(EventError::SnapshotUnavailable.into()),
    }
}

/// One prompt and the registry terminal it was opened on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Row {
    pub terminal: TerminalId,
    pub record: PromptRecord,
}

fn row_key(runtime: RuntimeId, prompt: &PromptId) -> ResourceKey {
    ResourceKey::new("agent_prompt", format!("{runtime}/{}", prompt.as_str()))
}

/// The stored prompt `prompt`, if one exists.
pub(super) fn prompt(
    snapshot: &Snapshot,
    runtime: RuntimeId,
    prompt: &PromptId,
) -> Result<Option<Versioned<Row>>, PromptError> {
    let key = row_key(runtime, prompt);
    let (row, version) = read::<Row>(snapshot, &key)?;
    Ok(row.map(|value| Versioned {
        value,
        version,
        key,
    }))
}

/// A new prompt row, continuing the version of any deleted prompt of the same identity.
pub(super) fn created(
    snapshot: &Snapshot,
    runtime: RuntimeId,
    row: Row,
) -> Result<Versioned<Row>, PromptError> {
    let key = row_key(runtime, &row.record.prompt_id);
    let (_, version) = read::<Row>(snapshot, &key)?;
    Ok(Versioned {
        value: row,
        version,
        key,
    })
}

/// One indexed prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Entry {
    pub prompt_id: PromptId,
    pub terminal: TerminalId,
    pub open: bool,
}

/// Every retained prompt of a runtime, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Index {
    pub entries: Vec<Entry>,
}

/// The runtime's prompt index.
pub(super) fn index(
    snapshot: &Snapshot,
    runtime: RuntimeId,
) -> Result<Versioned<Index>, PromptError> {
    let key = ResourceKey::new("agent_prompt_index", runtime.to_string());
    let (index, version) = read::<Index>(snapshot, &key)?;
    Ok(Versioned {
        value: index.unwrap_or_default(),
        version,
        key,
    })
}

impl Index {
    /// Open prompts of `terminal`, oldest first.
    pub(super) fn open_on(&self, terminal: TerminalId) -> impl Iterator<Item = &PromptId> {
        self.entries
            .iter()
            .filter(move |entry| entry.open && entry.terminal == terminal)
            .map(|entry| &entry.prompt_id)
    }

    pub(super) fn add(&mut self, prompt_id: PromptId, terminal: TerminalId) {
        self.entries.push(Entry {
            prompt_id,
            terminal,
            open: true,
        });
    }

    /// Marks `prompt` closed and drops the oldest closed prompts of its terminal beyond
    /// the retained bound, returning the dropped identities.
    pub(super) fn closed(&mut self, prompt: &PromptId, terminal: TerminalId) -> Vec<PromptId> {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| &entry.prompt_id == prompt)
        {
            entry.open = false;
        }
        let closed = self
            .entries
            .iter()
            .filter(|entry| !entry.open && entry.terminal == terminal)
            .count();
        let mut excess = closed.saturating_sub(CLOSED_LIMIT);
        let mut dropped = Vec::new();
        self.entries.retain(|entry| {
            let drop = excess > 0 && !entry.open && entry.terminal == terminal;
            if drop {
                excess = excess.saturating_sub(1);
                dropped.push(entry.prompt_id.clone());
            }
            !drop
        });
        dropped
    }
}

/// The index write after `prompt` closed, plus the deletion of every row it dropped.
pub(super) fn close_in_index(
    snapshot: &Snapshot,
    (runtime, terminal): (RuntimeId, TerminalId),
    prompt: &PromptId,
) -> Result<Vec<CasWrite>, PromptError> {
    let mut index = index(snapshot, runtime)?;
    let dropped = index.value.closed(prompt, terminal);
    let mut writes = vec![index.write()?];
    for id in &dropped {
        if let Some(row) = self::prompt(snapshot, runtime, id)? {
            writes.push(row.delete());
        }
    }
    Ok(writes)
}
