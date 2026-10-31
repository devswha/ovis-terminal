//! Concurrent operation admission and deduplication.

use std::collections::HashMap;
use std::io::Write as _;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, LazyLock, Mutex};

use ovis_terminal_core::events::{EventError, Filter, SnapshotProducer};
use ovis_terminal_core::ops::{OperationProducer, OperationSnapshot};
use ovis_terminal_store::{Snapshot, Store, StoreOptions};
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    Coordinator, Fixture, Operations, Request, TestResult, context, identity, json, request,
};

static PAUSES: LazyLock<Mutex<HashMap<String, Pause>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone)]
struct Pause {
    barrier: Arc<Barrier>,
    remaining: Arc<AtomicUsize>,
}

#[derive(Serialize)]
struct PausedSnapshot {
    token: String,
    snapshot: OperationSnapshot,
}

impl<'de> Deserialize<'de> for PausedSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            token: String,
            snapshot: OperationSnapshot,
        }

        let wire = Wire::deserialize(deserializer)?;
        let pause = {
            let pauses = PAUSES.lock().map_err(serde::de::Error::custom)?;
            pauses.get(&wire.token).cloned()
        };
        if let Some(pause) = pause {
            let mut current = pause.remaining.load(Ordering::SeqCst);
            let claimed = loop {
                let Some(next) = current.checked_sub(1) else {
                    break false;
                };
                match pause.remaining.compare_exchange_weak(
                    current,
                    next,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break true,
                    Err(observed) => current = observed,
                }
            };
            if claimed {
                pause.barrier.wait();
            }
        }
        Ok(Self {
            token: wire.token,
            snapshot: wire.snapshot,
        })
    }
}

impl AsRef<OperationSnapshot> for PausedSnapshot {
    fn as_ref(&self) -> &OperationSnapshot {
        &self.snapshot
    }
}

struct PausingProducer {
    token: String,
    inner: OperationProducer,
}

impl SnapshotProducer for PausingProducer {
    type Sections = PausedSnapshot;

    fn capture(&self, snapshot: &Snapshot, _filter: &Filter) -> Result<Self::Sections, EventError> {
        Ok(PausedSnapshot {
            token: self.token.clone(),
            snapshot: self.inner.capture(snapshot)?,
        })
    }
}

fn paused_core(
    fixture: &Fixture,
    readers: usize,
) -> Result<Coordinator<super::OperationEvent, PausingProducer>, Box<dyn std::error::Error>> {
    let identity = identity(1)?;
    let token = uuid::Uuid::now_v7().to_string();
    PAUSES.lock()?.insert(
        token.clone(),
        Pause {
            barrier: Arc::new(Barrier::new(readers)),
            remaining: Arc::new(AtomicUsize::new(readers)),
        },
    );
    Ok(Coordinator::new(
        Store::open(&fixture.0, StoreOptions::default())?,
        identity,
        PausingProducer {
            token,
            inner: OperationProducer::new(identity.runtime_id()),
        },
    ))
}

#[test]
fn concurrent_identical_accepts_return_one_record_and_grant_one_effect() -> TestResult {
    // Given eight callers forced to capture the same empty durable ledger.
    let fixture = Fixture::new();
    let core = paused_core(&fixture, 8)?;
    let req = request("same")?;
    let accepted = Arc::new(Barrier::new(8));
    let effect = fixture.0.join("admission-effect");

    // When all callers accept identical work, then race for execution permission.
    let records = std::thread::scope(|scope| {
        let mut callers = Vec::new();
        for _ in 0..8 {
            let accepted = Arc::clone(&accepted);
            let effect = effect.clone();
            let core = &core;
            let req = &req;
            callers.push(scope.spawn(move || {
                let operations = Operations::new(core, context(1).expect("context"));
                let record = operations.accept(req);
                accepted.wait();
                if record.is_ok()
                    && let Ok(permit) = operations.start(req.op_id())
                {
                    let mut output = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(effect)
                        .expect("effect file");
                    output.write_all(b"effect\n").expect("effect write");
                    operations
                        .commit(permit, json!({"effect":true}))
                        .expect("completion");
                }
                record
            }));
        }
        callers
            .into_iter()
            .map(|caller| caller.join().expect("caller joined"))
            .collect::<Vec<_>>()
    });

    // Then every admission returns the same inspectable record and one effect occurs.
    let effect_bytes = std::fs::read(effect)?;
    core.close()?;
    let records = records.into_iter().collect::<Result<Vec<_>, _>>()?;
    assert!(
        records
            .windows(2)
            .all(|pair| matches!(pair, [left, right] if left == right))
    );
    assert_eq!(effect_bytes, b"effect\n");
    Ok(())
}

#[test]
fn concurrent_changed_payloads_return_the_winner_and_seven_reuse_errors() -> TestResult {
    // Given eight different payloads forced to capture the same empty durable ledger.
    let fixture = Fixture::new();
    let core = paused_core(&fixture, 8)?;
    let requests = (0..8)
        .map(|index| {
            Request::new(
                request("seed")?.op_id().clone(),
                "terminal.run".into(),
                json!({"argv":[format!("payload-{index}")]}),
            )
            .map_err(Into::into)
        })
        .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;

    // When all different payloads race for the same operation identity.
    let outcomes = std::thread::scope(|scope| {
        let callers = requests
            .iter()
            .map(|req| {
                let core = &core;
                scope.spawn(move || Operations::new(core, context(1).expect("context")).accept(req))
            })
            .collect::<Vec<_>>();
        callers
            .into_iter()
            .map(|caller| caller.join().expect("caller joined"))
            .collect::<Vec<_>>()
    });

    // Then one record wins and every changed payload returns op_reused with that receipt.
    core.close()?;
    let winner = outcomes
        .iter()
        .find_map(|outcome| outcome.as_ref().ok())
        .ok_or("missing winning record")?;
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    for error in outcomes.iter().filter_map(|outcome| outcome.as_ref().err()) {
        assert_eq!(error.kind(), "op_reused");
        assert_eq!(error.receipt(), Some(winner));
    }
    Ok(())
}
