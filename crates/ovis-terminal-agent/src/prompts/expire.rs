//! Expiry of an ended program's open prompts inside the caller's commit unit.

use std::collections::BTreeMap;

use ovis_terminal_protocol::schema::ids::PromptId;
use ovis_terminal_protocol::schema::methods::status::{PromptRecord, PromptState};
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_store::{CasWrite, Snapshot};

use super::{PromptError, PromptService, store};
use crate::status::ProcessOwner;

impl PromptService {
    /// Expires every open prompt `owner`'s program opened, returning the closed records
    /// and the writes that commit them; the caller commits one `prompt.closed` event per
    /// closed record, in order, the first at `first_seq`.
    pub(crate) fn expire_program(
        &self,
        snapshot: &Snapshot,
        (owner, first_seq): (ProcessOwner, EventSeq),
    ) -> Result<(Vec<PromptRecord>, Vec<CasWrite>), PromptError> {
        let mut index = store::index(snapshot, self.runtime)?;
        let open: Vec<PromptId> = index.value.open_on(owner.terminal).cloned().collect();
        let mut seq = first_seq;
        let mut closed = Vec::new();
        let mut writes = BTreeMap::new();
        let mut dropped = Vec::new();
        for id in open {
            let Some(mut stored) = store::prompt(snapshot, self.runtime, &id)? else {
                continue;
            };
            if stored.value.record.process_incarnation != owner.process {
                continue;
            }
            stored.value.record.close(PromptState::Expired, seq);
            seq = EventSeq::new(seq.get().saturating_add(1));
            closed.push(stored.value.record.clone());
            writes.insert(id.clone(), stored.write()?);
            dropped.extend(index.value.closed(&id, owner.terminal));
        }
        for id in dropped {
            if let Some(row) = store::prompt(snapshot, self.runtime, &id)? {
                writes.insert(id, row.delete());
            }
        }
        let mut writes: Vec<CasWrite> = writes.into_values().collect();
        if !closed.is_empty() {
            writes.push(index.write()?);
        }
        Ok((closed, writes))
    }
}
