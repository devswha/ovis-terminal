//! Principal-bound operation admission and durable row updates.

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::server::{OpRecord, OpState};
use ovis_terminal_protocol::schema::scalar::{OpenEnum, TimestampMs};
use ovis_terminal_store::{CasWrite, OpId as StoreOpId, Payload, Version};
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use super::ledger::{Entry, OperationKey, Row, index_key};
use super::{OP_RETENTION_MS, OperationError, OperationEvent, OperationSnapshot, Request};
use crate::events::{Commit, Coordinator, EventData, ResourceId, SnapshotProducer};
use crate::registry::RuntimeIdentity;

/// Authenticated principal, owning runtime and boundary-supplied clock reading.
#[derive(Debug, Clone)]
pub struct Context {
    pub(super) principal: PrincipalId,
    pub(super) identity: RuntimeIdentity,
    pub(super) now: TimestampMs,
}

impl Context {
    /// Captures the authenticated identity and clock for one batch of operations.
    pub const fn new(principal: PrincipalId, identity: RuntimeIdentity, now: TimestampMs) -> Self {
        Self {
            principal,
            identity,
            now,
        }
    }

    /// Captures a later clock reading without changing authentication.
    #[must_use]
    pub const fn at(mut self, now: TimestampMs) -> Self {
        self.now = now;
        self
    }
}

/// Non-cloneable permission granted only after the running intent commits.
#[derive(Debug)]
pub struct RunningOperation {
    pub(super) op_id: OpId,
    pub(super) context: Context,
}

/// Blocking-worker operation boundary over the shared commit coordinator.
#[derive(Debug)]
pub struct Operations<'a, D, P> {
    pub(super) coordinator: &'a Coordinator<D, P>,
    pub(super) context: Context,
}

impl<'a, D, P> Operations<'a, D, P> {
    /// Binds mutations and inspection to one authenticated principal and lifetime.
    pub const fn new(coordinator: &'a Coordinator<D, P>, context: Context) -> Self {
        Self {
            coordinator,
            context,
        }
    }
}

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
        let mut snapshot = self.snapshot()?;
        let key = OperationKey::new(&self.context.principal, &request.op_id)?;
        if let Some(row) = snapshot.rows.iter().find(|row| row.key == key) {
            self.retained(&row.entry)?;
            if row.entry.record.method != request.method || row.entry.params != request.params {
                return Err(OperationError::Reused(Box::new(row.entry.record.clone())));
            }
            return self.get(&request.op_id);
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
        let entry = Entry {
            principal: self.context.principal.clone(),
            params: request.params.clone(),
            accepted_at: self.context.now,
            boot: self.context.identity.server_boot_id(),
            record: OpRecord::accepted(request.op_id.clone(), request.method.clone()),
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
        self.persist(entry.record, writes)
    }

    pub(super) fn snapshot(&self) -> Result<OperationSnapshot, OperationError> {
        let snapshot = self.coordinator.read_snapshot()?;
        if snapshot.cursor.runtime_id != self.context.identity.runtime_id()
            || snapshot.cursor.machine_id != self.context.identity.machine_id()
        {
            return Err(OperationError::InvalidLedger);
        }
        Ok(snapshot.sections.as_ref().clone())
    }

    pub(super) const fn retained(&self, entry: &Entry) -> Result<(), OperationError> {
        if self
            .context
            .now
            .get()
            .saturating_sub(entry.accepted_at.get())
            >= OP_RETENTION_MS.get()
        {
            return Err(OperationError::Expired);
        }
        Ok(())
    }

    pub(super) fn current(&self, op: &OpId) -> Result<Row, OperationError> {
        let snapshot = self.snapshot()?;
        let key = OperationKey::new(&self.context.principal, op)?;
        let row = snapshot.rows.into_iter().find(|row| row.key == key);
        let Some(row) = row else {
            let minted = super::request::minted_at(op)?;
            let floor = snapshot
                .index
                .floor
                .get()
                .max(self.context.now.get().saturating_sub(OP_RETENTION_MS.get()));
            return if minted.get() < floor {
                Err(OperationError::Expired)
            } else {
                Err(OperationError::NotFound)
            };
        };
        self.retained(&row.entry)?;
        Ok(row)
    }

    pub(super) fn update(&self, row: Row) -> Result<OpRecord, OperationError> {
        let bytes = serde_json::to_vec(&row.entry)?;
        if bytes.len() > 16 << 10 {
            return Err(OperationError::TooLarge);
        }
        let write = CasWrite::new(
            row.key.resource(self.context.identity.runtime_id()),
            Version::new(row.version.get()),
            Some(Payload::from(bytes)),
        );
        self.persist(row.entry.record, vec![write])
    }

    fn persist(
        &self,
        mut record: OpRecord,
        writes: Vec<CasWrite>,
    ) -> Result<OpRecord, OperationError> {
        let event = OperationEvent::from_record(&record);
        let mut commit = Commit::new(
            StoreOpId::new(Uuid::now_v7().to_string()),
            D::from(event),
            vec![ResourceId::Machine(self.context.identity.machine_id())],
        );
        for write in writes {
            commit = commit.write(write);
        }
        record.updated_seq = self
            .coordinator
            .commit(commit)
            .map_err(|source| OperationError::Transition {
                op_id: record.op_id.clone(),
                source,
            })?
            .seq;
        Ok(record)
    }

    /// Inspects state, retaining the uncertainty of a previous core's running intent.
    ///
    /// # Errors
    ///
    /// Returns `not_found`, `op_expired` or typed persistence/recovery failures.
    pub fn get(&self, op: &OpId) -> Result<OpRecord, OperationError> {
        let mut row = self.current(op)?;
        match row.entry.record.state {
            OpenEnum::Known(OpState::Running)
                if row.entry.boot != self.context.identity.server_boot_id() =>
            {
                row.entry.record.state = OpenEnum::Known(OpState::OutcomeUnknown);
                self.update(row)
            }
            _ => Ok(row.entry.record),
        }
    }
}
