//! The shared commit boundary against the real SQLite writer (plan todo 42).

use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;

use ovis_terminal_core::events::{
    Commit, Coordinator, EventData, EventError, Filter, Limits, ResourceId, SnapshotProducer,
};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, IdentityError, MachineId, RuntimeId, RuntimeIdentity, ServerBootId,
    TerminalId,
};
use ovis_terminal_core::subscriptions::LossRecord;
use ovis_terminal_store::{
    CasWrite, EventPage, OpId, Payload, ResourceKey, Revision, Snapshot, Store, StoreError,
    StoreOptions, Version,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

type TestResult = Result<(), Box<dyn Error>>;

#[path = "../src/commit/qa/install.rs"]
mod install;
#[path = "../src/commit/qa/pages.rs"]
mod pages;
#[path = "../src/commit/qa/remote.rs"]
mod remote;
#[path = "../src/commit/qa/sources.rs"]
mod sources;
#[path = "../src/commit/qa/storage.rs"]
mod storage;

/// A record naming the source that committed it and its position in that source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Probe {
    Wrote { source: String, n: u64 },
    Lost(LossRecord),
}

impl From<LossRecord> for Probe {
    fn from(loss: LossRecord) -> Self {
        Self::Lost(loss)
    }
}

impl EventData for Probe {
    fn event_type(&self) -> &'static str {
        match self {
            Self::Wrote { .. } => "probe.wrote",
            Self::Lost(_) => "subscription.lost",
        }
    }
}

/// The committed sequences visible at the snapshot's revision, plus filler bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Captured {
    seqs: Vec<u64>,
    filler: String,
}

#[derive(Debug, Default)]
struct Producer {
    filler: usize,
}

impl SnapshotProducer for Producer {
    type Sections = Captured;

    fn capture(&self, snapshot: &Snapshot, _filter: &Filter) -> Result<Captured, EventError> {
        let seqs = match snapshot.events_after(Revision::ZERO, 10_000)? {
            EventPage::Events(events) => events.iter().map(|event| event.seq.get()).collect(),
            _ => return Err(EventError::SnapshotUnavailable),
        };
        Ok(Captured {
            seqs,
            filler: "f".repeat(self.filler),
        })
    }
}

type Core = Coordinator<Probe, Producer>;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("ovis-commit-{}", Uuid::now_v7())))
    }

    fn open(&self, options: StoreOptions) -> Result<Store, StoreError> {
        Store::open(&self.0, options)
    }

    fn core(
        &self,
        (options, producer): (StoreOptions, Producer),
    ) -> Result<Arc<Core>, Box<dyn Error>> {
        Ok(Arc::new(Coordinator::new(
            self.open(options)?,
            identity()?,
            producer,
        )))
    }

    fn limited(&self, limits: Limits, producer: Producer) -> Result<Arc<Core>, Box<dyn Error>> {
        let core = Coordinator::new(self.open(StoreOptions::default())?, identity()?, producer);
        Ok(Arc::new(core.with_limits(limits)?))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.0);
        assert!(removed.is_ok(), "commit fixture cleanup: {removed:?}");
    }
}

fn identity() -> Result<RuntimeIdentity, IdentityError> {
    Ok(RuntimeIdentity::new(
        HostIdentity::new(
            MachineId::new(Uuid::from_u128(1))?,
            HostBootId::try_from(1)?,
        ),
        RuntimeId::try_from(1)?,
        ServerBootId::try_from(1)?,
    ))
}

fn wrote(source: &str, n: u64) -> Result<Commit<Probe>, Box<dyn Error>> {
    Ok(Commit::new(
        OpId::new(Uuid::now_v7().to_string()),
        Probe::Wrote {
            source: source.to_owned(),
            n,
        },
        vec![ResourceId::Terminal(TerminalId::try_from(1)?)],
    ))
}

fn with_state(commit: Commit<Probe>, (key, bytes): (&str, usize)) -> Commit<Probe> {
    commit.write(CasWrite::new(
        ResourceKey::new("probe", key),
        Version::ABSENT,
        Some(Payload::from(vec![b'x'; bytes])),
    ))
}

fn close(core: Arc<Core>) -> TestResult {
    Arc::try_unwrap(core)
        .map_err(|_| "core still shared")?
        .close()?;
    Ok(())
}
