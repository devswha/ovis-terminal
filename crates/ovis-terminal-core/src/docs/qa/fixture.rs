//! A private store root, the real coordinator, and `doc.*` driven through the operation ledger.

use std::error::Error;
use std::path::PathBuf;

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_protocol::domain::documents_retention::RetentionFloors;
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::DocName;
use ovis_terminal_protocol::schema::methods::doc::{DocRecord, DocSetParams};
use ovis_terminal_protocol::schema::methods::server::OpRecord;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_store::{Snapshot, Store, StoreOptions};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::docs::{DocumentsRetentionEvent, read, set_unit};
use crate::events::{Coordinator, EventError, Filter, SnapshotProducer};
use crate::ops::{
    Context, OperationError, OperationProducer, OperationSnapshot, Operations, Request,
};
use crate::registry::{
    HostBootId, HostIdentity, MachineId, RuntimeId, RuntimeIdentity, ServerBootId,
};
use crate::retention::floors;

pub(super) type TestResult<T = ()> = Result<T, Box<dyn Error>>;
pub(super) type Core = Coordinator<DocumentsRetentionEvent, Producer>;

/// Clock reading of the fixture's virtual "now".
pub(super) const NOW_MS: u64 = 1_800_000_000_000;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Sections {
    pub operations: OperationSnapshot,
    pub floors: RetentionFloors,
}

impl AsRef<OperationSnapshot> for Sections {
    fn as_ref(&self) -> &OperationSnapshot {
        &self.operations
    }
}

#[derive(Debug)]
pub(super) struct Producer(OperationProducer);

impl SnapshotProducer for Producer {
    type Sections = Sections;

    fn capture(&self, snapshot: &Snapshot, _: &Filter) -> Result<Sections, EventError> {
        Ok(Sections {
            operations: self.0.capture(snapshot)?,
            floors: floors(snapshot).map_err(|_| EventError::SnapshotUnavailable)?,
        })
    }
}

pub(super) struct Root(PathBuf);

impl Root {
    pub(super) fn new() -> TestResult<Self> {
        let path = std::env::temp_dir().join(format!("ovt50-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }

    pub(super) fn open(&self, options: StoreOptions) -> TestResult<Core> {
        let identity = identity()?;
        Ok(Coordinator::new(
            Store::open(&self.0, options)?,
            identity,
            Producer(OperationProducer::new(identity.runtime_id())),
        ))
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.0);
        assert!(removed.is_ok(), "cleanup: {removed:?}");
    }
}

pub(super) fn identity() -> TestResult<RuntimeIdentity> {
    let machine: MachineId = "machine:0191f1a2-0000-7000-8000-000000000050".parse()?;
    Ok(RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1)?),
        RuntimeId::try_from(50)?,
        ServerBootId::try_from(1)?,
    ))
}

/// A `UUIDv7` minted `age_ms` before [`NOW_MS`].
pub(super) fn op_minted(age_ms: u64) -> TestResult<OpId> {
    let at = NOW_MS.checked_sub(age_ms).ok_or("age before epoch")?;
    let stamp = uuid::Timestamp::from_unix(
        uuid::NoContext,
        at / 1000,
        u32::try_from(at % 1000)?
            .checked_mul(1_000_000)
            .ok_or("nanos")?,
    );
    Ok(OpId::new(uuid::Uuid::new_v7(stamp).to_string()))
}

fn operations(core: &Core) -> TestResult<Operations<'_, DocumentsRetentionEvent, Producer>> {
    let context = Context::new(
        PrincipalId::parse("owner")?,
        identity()?,
        TimestampMs::new(NOW_MS),
    );
    Ok(Operations::new(core, context))
}

/// Accepts and runs one `doc.set` exactly as dispatch does: the op id is split off the params.
pub(super) fn set(
    core: &Core,
    op_id: &OpId,
    mut params: Value,
) -> TestResult<Result<OpRecord, OperationError>> {
    let object = params.as_object_mut().ok_or("params object")?;
    object.insert("op_id".to_owned(), serde_json::to_value(op_id)?);
    let typed: DocSetParams = serde_json::from_value(params.clone())?;
    let mut canonical = params;
    canonical
        .as_object_mut()
        .ok_or("params object")?
        .remove("op_id");
    let operations = operations(core)?;
    let request = match Request::new(op_id.clone(), "doc.set".to_owned(), canonical) {
        Ok(request) => request,
        Err(error) => return Ok(Err(error)),
    };
    let accepted = match operations.accept(&request) {
        Ok(record) => record,
        Err(error) => return Ok(Err(error)),
    };
    if accepted.result.is_some() || accepted.error.is_some() {
        return Ok(Ok(accepted));
    }
    let permit = operations.start(request.op_id())?;
    Ok(operations.commit_unit(permit, |snapshot| Ok(set_unit(typed, snapshot))))
}

/// Reads one document from a committed snapshot, as `doc.get` does: no receipt, no event.
pub(super) fn get(core: &Core, name: &str) -> TestResult<DocRecord> {
    let name = DocName::new(name.to_owned());
    Ok(core.read_view(|snapshot| Ok::<_, EventError>(read(snapshot, &name)))??)
}

/// The JSON value at `pointer`, or null.
pub(super) fn at<'a>(value: &'a Value, pointer: &str) -> &'a Value {
    value.pointer(pointer).unwrap_or(&Value::Null)
}

/// The error kind retained with a failed operation.
pub(super) fn failed_kind(record: &OpRecord) -> Option<String> {
    record
        .error
        .as_ref()
        .map(|error| error.kind.token().to_owned())
}
