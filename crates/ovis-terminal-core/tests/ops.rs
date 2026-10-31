//! Durable operations against the real coordinator and SQLite writer.

use std::error::Error;
use std::path::PathBuf;

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::{Context, OperationEvent, OperationProducer, Operations, Request};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RuntimeId, RuntimeIdentity, ServerBootId,
};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::server::{CancelOutcome, OpState};
use ovis_terminal_protocol::schema::scalar::{OpenEnum, TimestampMs};
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::json;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn Error>>;
type Core = Coordinator<OperationEvent, OperationProducer>;

#[path = "../src/ops/qa/admission.rs"]
mod admission;
#[path = "../src/ops/qa/adversarial.rs"]
mod adversarial;
#[path = "../src/ops/qa/atomic.rs"]
mod atomic;
#[path = "../src/ops/qa/bounds.rs"]
mod bounds;
#[path = "../src/ops/qa/concurrency.rs"]
mod concurrency;
#[path = "../src/ops/qa/digest.rs"]
mod digest;
#[path = "../src/ops/qa/fairness.rs"]
mod fairness;
#[cfg(unix)]
#[path = "../src/ops/qa/holder.rs"]
pub mod holder;
#[path = "../src/ops/qa/inspection.rs"]
mod inspection;
#[path = "../src/ops/qa/ownership.rs"]
mod ownership;
#[path = "../src/ops/qa/pending.rs"]
mod pending;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("ovis-ops-{}", Uuid::now_v7())))
    }

    fn open(&self, boot: u64) -> Result<Core, Box<dyn Error>> {
        let identity = identity(boot)?;
        Ok(Coordinator::new(
            Store::open(&self.0, StoreOptions::default())?,
            identity,
            OperationProducer::new(identity.runtime_id()),
        ))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.0);
        assert!(removed.is_ok(), "fixture cleanup: {removed:?}");
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

fn context(boot: u64) -> Result<Context, Box<dyn Error>> {
    Ok(Context::new(
        PrincipalId::parse("operator")?,
        identity(boot)?,
        TimestampMs::new(100_000_000),
    ))
}

fn request(argv: &str) -> Result<Request, Box<dyn Error>> {
    Ok(Request::new(
        OpId::new("000005f5-e100-7000-8000-000000000001".into()),
        "terminal.run".into(),
        json!({"argv":[argv], "env":{"B":"2","A":"1"}}),
    )?)
}

#[test]
fn happy() -> TestResult {
    // Given durable acceptance before an actual holder effect on Unix.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let operations = Operations::new(&core, context(1)?);
    let req = request("one")?;
    let accepted = operations.accept(&req)?;
    assert_eq!(accepted.state, OpenEnum::Known(OpState::Accepted));
    #[cfg(unix)]
    let receipt = holder::perform(&operations)?;
    #[cfg(not(unix))]
    let receipt = {
        let permit = operations.start(req.op_id())?;
        std::fs::write(fixture.0.join("effect"), b"one effect\n")?;
        operations.commit(permit, json!({"bytes":"11"}))?
    };

    // When the committed operation is retried, including after a new core boot.
    let repeated = operations.accept(&req)?;
    assert_eq!(receipt, repeated);
    assert!(operations.start(req.op_id()).is_err());
    core.close()?;
    let reopened = fixture.open(2)?;
    let operations = Operations::new(&reopened, context(2)?);
    let retained = operations.accept(&req)?;

    // Then the same inspectable receipt survives and execution is never granted twice.
    assert_eq!(receipt, retained);
    #[cfg(not(unix))]
    assert_eq!(std::fs::read(fixture.0.join("effect"))?, b"one effect\n");
    assert!(operations.start(req.op_id()).is_err());
    assert_eq!(operations.get(req.op_id())?, receipt);
    println!("receipt={}", serde_json::to_string(&retained)?);
    reopened.close()?;
    Ok(())
}

#[test]
fn failure() -> TestResult {
    // Given an operation whose intent committed but whose effect was not acknowledged.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let operations = Operations::new(&core, context(1)?);
    let req = request("original")?;
    operations.accept(&req)?;
    let _interrupted = operations.start(req.op_id())?;
    std::fs::write(fixture.0.join("uncertain-effect"), b"written before ack")?;
    core.close()?;

    // When the core restarts and retries changed, expired and uncertain requests.
    let core = fixture.open(2)?;
    let operations = Operations::new(&core, context(2)?);
    let changed = operations
        .accept(&request("changed")?)
        .expect_err("changed argv");
    assert_eq!(changed.kind(), "op_reused");
    let expired = Request::new(
        OpId::new("00000000-0001-7000-8000-000000000001".into()),
        "terminal.run".into(),
        json!({"argv":["old"]}),
    )?;
    assert_eq!(
        operations.accept(&expired).expect_err("old ID").kind(),
        "op_expired"
    );

    // Then uncertain delivery is durable, queryable and never re-executed.
    let unknown = operations.get(req.op_id())?;
    assert_eq!(unknown.state, OpenEnum::Known(OpState::OutcomeUnknown));
    assert_eq!(
        operations.start(req.op_id()).expect_err("no replay").kind(),
        "outcome_unknown"
    );
    let cancelled = operations.cancel(req.op_id())?;
    assert_eq!(cancelled.outcome, OpenEnum::Known(CancelOutcome::Pending));
    assert_eq!(operations.get(req.op_id())?.state, unknown.state);
    println!(
        "reused={}; expired=op_expired; unknown={}",
        changed.kind(),
        serde_json::to_string(&unknown)?
    );
    core.close()?;
    Ok(())
}
