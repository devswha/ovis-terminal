//! Admitting a request once per principal and identity, optionally already cancelled.

use ovis_terminal_protocol::schema::methods::server::{OpRecord, OpState};
use ovis_terminal_protocol::schema::scalar::{OpenEnum, TimestampMs};
use ovis_terminal_store::{CasWrite, Payload, Version};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::ledger::{Entry, OperationKey, index_key};
use super::{
    OP_RETENTION_MS, OperationError, OperationEvent, OperationSnapshot, Operations, Request,
};
use crate::events::{EventData, SnapshotProducer};

impl<D, P> Operations<'_, D, P>
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned,
    P: SnapshotProducer,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    /// Accepts once, or returns the prior outcome for the same method and canonical params.
    ///
    /// The key is `principal/runtime/op_id`; the fingerprint binds method and params,
    /// so changing methods cannot silently retarget an inspectable identity.
    ///
    /// # Errors
    ///
    /// Returns `op_reused`, `op_expired`, bounded admission or persistence errors.
    pub fn accept(&self, request: &Request) -> Result<OpRecord, OperationError> {
        match self.admit(request, Admission::Runnable)? {
            Admitted::New(record) => Ok(*record),
            Admitted::Retained => self.get(&request.op_id),
        }
    }

    pub(super) fn admit(
        &self,
        request: &Request,
        admission: Admission,
    ) -> Result<Admitted, OperationError> {
        loop {
            match self.accept_once(request, admission) {
                Err(error) if error.is_conflict() => {}
                outcome => return outcome,
            }
        }
    }

    fn accept_once(
        &self,
        request: &Request,
        admission: Admission,
    ) -> Result<Admitted, OperationError> {
        let mut snapshot = self.snapshot()?;
        let key = OperationKey::new(&self.context.principal, &request.op_id)?;
        if let Some(row) = snapshot.rows.iter().find(|row| row.key == key) {
            self.retained(&row.entry)?;
            if row.entry.record.method != request.method || row.entry.params != request.params {
                return Err(OperationError::Reused(Box::new(row.entry.record.clone())));
            }
            return Ok(Admitted::Retained);
        }
        let floor = snapshot
            .index
            .floor
            .get()
            .max(self.context.now.get().saturating_sub(OP_RETENTION_MS.get()));
        if request.minted_at.get() < floor {
            return Err(OperationError::Expired);
        }
        if request.minted_at > self.context.now {
            return Err(OperationError::InvalidParams);
        }
        let mut writes = Vec::new();
        snapshot.rows.retain(|row| {
            let keep = row.entry.accepted_at.get() >= floor;
            if !keep {
                writes.push(CasWrite::new(
                    row.key.resource(self.context.identity.runtime_id()),
                    Version::new(row.version.get()),
                    None,
                ));
            }
            keep
        });
        if snapshot.rows.len() >= 1024 {
            return Err(OperationError::Capacity);
        }
        snapshot.index.floor = TimestampMs::new(floor);
        snapshot.index.keys = snapshot.rows.iter().map(|row| row.key.clone()).collect();
        snapshot.index.keys.push(key.clone());
        let accepted = OpRecord::accepted(request.op_id.clone(), request.method.clone());
        let mut entry = Entry {
            principal: self.context.principal.clone(),
            params: request.params.clone(),
            accepted_at: self.context.now,
            boot: self.context.identity.server_boot_id(),
            record: accepted.clone(),
        };
        // A cancelled admission still publishes its acceptance, in the same transaction.
        let prior = match admission {
            Admission::Runnable => None,
            Admission::Cancelled => {
                entry.record.state = OpenEnum::Known(OpState::Cancelled);
                Some(accepted)
            }
        };
        writes.push(CasWrite::new(
            index_key(self.context.identity.runtime_id()),
            Version::new(snapshot.version.get()),
            Some(Payload::from(serde_json::to_vec(&snapshot.index)?)),
        ));
        // A tombstoned key cannot pass this CAS, even if a client forges a newer clock.
        writes.push(CasWrite::new(
            key.resource(self.context.identity.runtime_id()),
            Version::ABSENT,
            Some(Payload::from(serde_json::to_vec(&entry)?)),
        ));
        self.persist((prior.as_ref(), entry.record), writes)
            .map(|record| Admitted::New(Box::new(record)))
    }
}

/// Whether a new identity may run, or its cancellation already reached it.
#[derive(Debug, Clone, Copy)]
pub(super) enum Admission {
    Runnable,
    Cancelled,
}

/// A newly committed record, or an equivalent one retained under the same identity.
#[derive(Debug)]
pub(super) enum Admitted {
    New(Box<OpRecord>),
    Retained,
}
