//! A ledger written before todo 78: bare SHA-256 bytes, no params copy.

use std::error::Error;
use std::path::PathBuf;

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_store::{
    CasWrite, OpId as StoreOpId, Payload, ResourceKey, ResourceLookup, Store, StoreOptions, Version,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::events::{Commit, Coordinator, EventError, ResourceId};
use crate::ops::{Context, OperationEvent, OperationProducer, Operations, Request};
use crate::registry::{
    HostBootId, HostIdentity, MachineId, RuntimeId, RuntimeIdentity, ServerBootId,
};

type TestResult = Result<(), Box<dyn Error>>;
type Core = Coordinator<OperationEvent, OperationProducer>;

const NOW: u64 = 100_000_000;
const LATER: u64 = NOW + 86_400_001;

struct Root(PathBuf);

impl Root {
    fn open(&self, boot: u64) -> Result<Core, Box<dyn Error>> {
        let identity = identity(boot)?;
        let store = Store::open(&self.0, StoreOptions::default())?;
        Ok(Coordinator::new(
            store,
            identity,
            OperationProducer::new(identity.runtime_id()),
        ))
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.0);
        assert!(removed.is_ok(), "cleanup: {removed:?}");
    }
}

fn identity(boot: u64) -> Result<RuntimeIdentity, Box<dyn Error>> {
    Ok(RuntimeIdentity::new(
        HostIdentity::new(
            MachineId::new(Uuid::from_u128(1))?,
            HostBootId::try_from(1)?,
        ),
        RuntimeId::try_from(1)?,
        ServerBootId::try_from(boot)?,
    ))
}

fn operations(
    core: &Core,
    (boot, now): (u64, u64),
) -> Result<Operations<'_, OperationEvent, OperationProducer>, Box<dyn Error>> {
    let context = Context::new(
        PrincipalId::parse("operator")?,
        identity(boot)?,
        TimestampMs::new(now),
    );
    Ok(Operations::new(core, context))
}

fn request((minted, sequence): (u64, u128), argv: &str) -> Result<Request, Box<dyn Error>> {
    let bits = (u128::from(minted) << 80) | (7_u128 << 76) | (2_u128 << 62) | sequence;
    let op_id = OpId::new(Uuid::from_u128(bits).to_string());
    Ok(Request::new(
        op_id,
        "terminal.run".into(),
        json!({"argv": [argv]}),
    )?)
}

fn resource(core: &Core, key: &ResourceKey) -> Result<ResourceLookup, EventError> {
    core.read_view(|snapshot| Ok::<_, EventError>(snapshot.resource(key)?))
}

/// Rewrites `op`'s row into the pre-78 shape and deletes its params copy.
fn downgrade(core: &Core, op: &OpId) -> TestResult {
    let operations = operations(core, (1, NOW))?;
    let row = operations.current(op)?;
    let runtime = identity(1)?.runtime_id();
    let hex = row.entry.params.digest.hex();
    let bytes = hex
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| Ok(u8::from_str_radix(std::str::from_utf8(pair)?, 16)?))
        .collect::<Result<Vec<u8>, Box<dyn Error>>>()?;
    let mut stored = serde_json::to_value(&row.entry)?;
    let fields = stored.as_object_mut().ok_or("row object")?;
    fields.remove("params_digest").ok_or("digest field")?;
    fields.remove("params_copy").ok_or("copy field")?;
    fields.insert("params".to_owned(), json!(bytes));
    let commit = Commit::new(
        StoreOpId::new(Uuid::now_v7().to_string()),
        OperationEvent::from_record(&row.entry.record),
        vec![ResourceId::Machine(identity(1)?.machine_id())],
    )
    .write(CasWrite::new(
        row.key.resource(runtime),
        Version::new(row.version.get()),
        Some(Payload::from(serde_json::to_vec(&stored)?)),
    ))
    .write(CasWrite::new(
        row.key.params_resource(runtime),
        Version::new(1),
        None,
    ));
    core.commit(commit)?;
    Ok(())
}

#[test]
fn pre_78_rows_still_deduplicate_and_are_rewritten_with_a_named_digest() -> TestResult {
    // Given: a ledger whose row was written before digests named their algorithm.
    let root = Root(std::env::temp_dir().join(format!("ovis-ops78-{}", Uuid::now_v7())));
    let core = root.open(1)?;
    let original = request((NOW, 1), "one")?;
    operations(&core, (1, NOW))?.accept(&original)?;
    downgrade(&core, original.op_id())?;
    let retained = operations(&core, (1, NOW))?.get(original.op_id())?;
    core.close()?;

    // When: the current build reopens it and the identity is retried.
    let core = root.open(2)?;
    let operations = operations(&core, (2, NOW))?;
    let identical = operations.accept(&request((NOW, 1), "one")?)?;
    let changed = operations
        .accept(&request((NOW, 1), "two")?)
        .expect_err("changed");
    let view = operations.params(original.op_id())?;

    // Then: an identical retry replays, a changed one is op_reused, and no copy is claimed.
    assert_eq!(identical, retained);
    assert_eq!(changed.kind(), "op_reused");
    assert_eq!(
        (view.digest, view.copy),
        (original.params.digest.clone(), None)
    );

    // And: the next transition rewrites the row with an algorithm-named digest.
    operations.cancel(original.op_id())?;
    let row = operations.current(original.op_id())?;
    let ResourceLookup::Live { value, .. } =
        resource(&core, &row.key.resource(identity(2)?.runtime_id()))?
    else {
        return Err("row not live".into());
    };
    let stored: Value = serde_json::from_slice(value.as_bytes())?;
    assert_eq!(stored.get("params"), None);
    assert_eq!(
        stored.pointer("/params_digest/algorithm"),
        Some(&json!("sha-256"))
    );
    Ok(core.close()?)
}

#[test]
fn eviction_deletes_a_copy_with_its_row_and_skips_rows_without_one() -> TestResult {
    // Given: a pre-78 row without a copy and a current row with one, both accepted at NOW.
    let root = Root(std::env::temp_dir().join(format!("ovis-ops78-{}", Uuid::now_v7())));
    let core = root.open(1)?;
    let (old, current) = (request((NOW, 1), "old")?, request((NOW, 2), "current")?);
    let first = operations(&core, (1, NOW))?;
    first.accept(&old)?;
    first.accept(&current)?;
    downgrade(&core, old.op_id())?;
    let copy = first
        .current(current.op_id())?
        .key
        .params_resource(identity(1)?.runtime_id());
    assert!(matches!(
        resource(&core, &copy)?,
        ResourceLookup::Live { .. }
    ));

    // When: a later acceptance evicts both rows past the retention window.
    let later = operations(&core, (1, LATER))?;
    later.accept(&request((LATER, 3), "later")?)?;

    // Then: both identities expired, and only the current row's copy left a tombstone.
    for op in [old.op_id(), current.op_id()] {
        assert_eq!(later.get(op).expect_err("evicted").kind(), "op_expired");
    }
    assert!(matches!(
        resource(&core, &copy)?,
        ResourceLookup::Tombstone { .. }
    ));
    Ok(core.close()?)
}
