//! An in-memory store with the durable outbox's contract.

use std::collections::BTreeMap;
use std::sync::Mutex;

use ovis_terminal_protocol::domain::agent_tree::OrphanNotice;
use ovis_terminal_protocol::schema::ids::MachineId;

use crate::orphans::{Outbox, OutboxError};

#[derive(Debug, Default)]
pub(super) struct Memory(pub(super) Mutex<BTreeMap<MachineId, Vec<OrphanNotice>>>);

fn poisoned<T>(_: T) -> OutboxError {
    OutboxError::new(("internal", "retry"), "poisoned")
}

impl Memory {
    fn read(&self, machine: &MachineId) -> Result<Vec<OrphanNotice>, OutboxError> {
        let outbox = self.0.lock().map_err(poisoned)?;
        let pending = outbox.get(machine).cloned().unwrap_or_default();
        drop(outbox);
        Ok(pending)
    }

    fn remove(&self, machine: &MachineId, notice: &OrphanNotice) -> Result<bool, OutboxError> {
        let mut outbox = self.0.lock().map_err(poisoned)?;
        let notices = outbox.entry(machine.clone()).or_default();
        let before = notices.len();
        notices.retain(|pending| pending != notice);
        let removed = notices.len() != before;
        drop(outbox);
        Ok(removed)
    }
}

impl Outbox for Memory {
    fn pending(
        &self,
        machine: &MachineId,
    ) -> impl std::future::Future<Output = Result<Vec<OrphanNotice>, OutboxError>> + Send {
        std::future::ready(self.read(machine))
    }

    fn acknowledge(
        &self,
        machine: &MachineId,
        notice: &OrphanNotice,
    ) -> impl std::future::Future<Output = Result<bool, OutboxError>> + Send {
        std::future::ready(self.remove(machine, notice))
    }
}
