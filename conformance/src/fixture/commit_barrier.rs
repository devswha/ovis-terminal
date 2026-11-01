//! `fixture.commit_barrier`: concurrent sources against one commit boundary.
//!
//! Terminal, registry and remote sources commit through the production boundary while a
//! 64 MiB snapshot is paged and a subscription installs, all against one coordinator and
//! its bounded SQLite writer. The reply reports what each observer saw; the scenario
//! checks they describe one boundary. Never advertised.

mod domain;
mod observe;

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use domain::{Ballast, Barrier};
use ovis_terminal_core::commit::{Boundary, CommitSource, Committed, PageTtl, Pager};
use ovis_terminal_core::events::{
    Commit, Coordinator, Limits, RemoteOrigin, RemoteSeq, ResourceId,
};
use ovis_terminal_core::registry::{MachineId, RuntimeId, TerminalId};
use ovis_terminal_core::subscriptions::{QueueLimits, Request, Selector, Start, Subscriptions};
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::common::NoParams;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_store::{OpId, Store, StoreOptions};
use serde_json::{Value, json};

use super::serve::{at, identity};
use crate::error::RunnerError;

/// The probe's method name; served beside the dispatcher, never advertised.
pub(crate) const METHOD: &str = "fixture.commit_barrier";

const PER_SOURCE: u64 = 40;
const SOURCES: [&str; 3] = ["terminal", "registry", "remote"];
const BOUND: Duration = Duration::from_secs(20);
const UPSTREAM_BASE: u64 = 1_000;

type Core = Coordinator<Barrier, Ballast>;

/// Runs the race on a blocking worker and answers its observations.
pub(crate) async fn call(root: &Path, request: &RpcRequest) -> Result<Value, BoundaryError> {
    let _: NoParams = decode_params(request).map_err(BoundaryError::from)?;
    let dir = root.join("commit-barrier");
    let raced = tokio::task::spawn_blocking(move || race(&dir)).await;
    raced
        .map_err(at("join the commit barrier race"))
        .and_then(|raced| raced)
        .map_err(|error| {
            BoundaryError::domain(ErrorKind::from(error.kind().to_owned()), false)
                .with_hint(error.hint())
        })
}

fn source(name: &str, n: u64) -> Result<CommitSource, RunnerError> {
    Ok(match name {
        "terminal" => CommitSource::Terminal {
            terminal_id: TerminalId::try_from(1).map_err(at("build a terminal id"))?,
        },
        "registry" => CommitSource::Registry,
        _ => CommitSource::Remote {
            origin: RemoteOrigin::new(
                "machine:0191f1a2-0000-7000-8000-000000000052"
                    .parse::<MachineId>()
                    .map_err(at("parse the remote machine"))?,
                RuntimeId::try_from(52).map_err(at("build the remote runtime"))?,
                RemoteSeq::new(EventSeq::new(n.saturating_add(UPSTREAM_BASE))),
            ),
        },
    })
}

fn record(name: &str, n: u64) -> Result<Commit<Barrier>, RunnerError> {
    let terminal = TerminalId::try_from(1).map_err(at("build a terminal id"))?;
    let data = Barrier::Wrote {
        source: name.to_owned(),
        n,
    };
    let op = OpId::new(uuid::Uuid::now_v7().to_string());
    Ok(Commit::new(op, data, vec![ResourceId::Terminal(terminal)]))
}

/// Commits `PER_SOURCE` records from `name`, signalling after the tenth.
fn committer(
    boundary: Boundary<Barrier, Ballast>,
    name: &'static str,
    started: mpsc::Sender<()>,
) -> std::thread::JoinHandle<Result<Vec<Committed>, RunnerError>> {
    std::thread::spawn(move || {
        let mut committed = Vec::new();
        for n in 1..=PER_SOURCE {
            let unit = record(name, n)?;
            let from = source(name, n)?;
            let done = boundary.commit(from, |_| Ok(unit));
            committed.push(done.map_err(at("commit through the boundary"))?);
            if n == 10 {
                started.send(()).map_err(at("signal the tenth commit"))?;
            }
        }
        Ok(committed)
    })
}

fn open_core(dir: &Path) -> Result<Arc<Core>, RunnerError> {
    let store = Store::open(dir, StoreOptions::default()).map_err(at("open the barrier store"))?;
    let mut limits = Limits::default();
    limits.snapshot_bytes = 160 << 20;
    limits.page_bytes = 1 << 20;
    limits.pinned_snapshots = 2;
    let core = Coordinator::new(store, identity()?, Ballast);
    Ok(Arc::new(
        core.with_limits(limits)
            .map_err(at("bound the coordinator"))?,
    ))
}

fn race(dir: &Path) -> Result<Value, RunnerError> {
    let core = open_core(dir)?;
    let boundary = Boundary::new(Arc::clone(&core), 4).map_err(at("share the boundary"))?;
    let limits = QueueLimits::v1().map_err(at("bound the subscriptions"))?;
    let hub = Subscriptions::start(Arc::clone(&core), identity()?, limits)
        .map_err(at("start the subscriptions"))?;
    let (started, ready) = mpsc::channel();
    let threads: Vec<_> = SOURCES
        .iter()
        .map(|name| committer(boundary.clone(), name, started.clone()))
        .collect();
    for _ in SOURCES {
        ready
            .recv_timeout(BOUND)
            .map_err(at("await the first commits"))?;
    }
    let pager = Pager::new(Arc::clone(&core), PageTtl::new(60_000).map_err(at("ttl"))?);
    let window = pager
        .open(observe::now()?)
        .map_err(at("pin the snapshot"))?;
    let installed = hub
        .subscribe(Request::new(Start::Snapshot, Selector::all()))
        .map_err(at("install the subscription"))?;
    let snapshot = observe::pages(&pager, &window)?;
    let mut sources = Vec::new();
    for thread in threads {
        let joined = thread
            .join()
            .map_err(|_| at("join a committer")("panicked"))?;
        sources.push(joined?);
    }
    let subscription = observe::subscription(&installed, PER_SOURCE.saturating_mul(3))?;
    drop((installed, pager, boundary));
    hub.shutdown().map_err(at("stop the subscriptions"))?;
    let core = Arc::try_unwrap(core).map_err(|_| at("release the coordinator")("still shared"))?;
    core.close().map_err(at("close the barrier store"))?;
    Ok(json!({
        "committed": committed(&sources),
        "snapshot": snapshot,
        "subscription": subscription,
    }))
}

/// Per-source counts, and how the local sequence covers what every source committed.
fn committed(sources: &[Vec<Committed>]) -> Value {
    let all: Vec<u64> = sources
        .iter()
        .flatten()
        .map(|c| c.local_seq().get())
        .collect();
    let distinct: BTreeSet<u64> = all.iter().copied().collect();
    let top = distinct.last().copied().unwrap_or(0);
    let missing = (1..=top).filter(|seq| !distinct.contains(seq)).count();
    let reordered: usize = sources
        .iter()
        .map(|records| {
            let backwards = |pair: &&[Committed]| match pair {
                [earlier, later] => earlier.local_seq() >= later.local_seq(),
                _ => false,
            };
            records.windows(2).filter(backwards).count()
        })
        .sum();
    let remote = sources.last().map(Vec::as_slice).unwrap_or_default();
    let upstream_kept = (1_u64..).zip(remote).all(|(n, c)| {
        c.remote().map(|origin| origin.seq.get()) == Some(n.saturating_add(UPSTREAM_BASE))
    });
    let counts: serde_json::Map<String, Value> = SOURCES
        .iter()
        .zip(sources)
        .map(|(name, records)| ((*name).to_owned(), json!(records.len().to_string())))
        .collect();
    json!({
        "by_source": counts,
        "total": all.len().to_string(),
        "missing": missing.to_string(),
        "duplicate": all.len().saturating_sub(distinct.len()).to_string(),
        "reordered": reordered.to_string(),
        "upstream_kept_apart": upstream_kept,
    })
}
