//! `op.cancel` racing the durable accept of the request it names.

use std::collections::BTreeMap;
use std::io::Write as _;

use serde_json::{Value, json};

use super::cancel_fixture::{Client, Fixture, TestResult, at, error_kind, op_id, request, set};

/// Occupies the runtime's only blocking worker, so every durable accept queues behind it.
struct Blocker {
    hold: std::sync::mpsc::Sender<()>,
    worker: tokio::task::JoinHandle<bool>,
}

impl Blocker {
    async fn occupy() -> TestResult<Self> {
        let (hold, parked) = std::sync::mpsc::channel::<()>();
        let (started, running) = tokio::sync::oneshot::channel();
        let worker =
            tokio::task::spawn_blocking(move || started.send(()).is_ok() && parked.recv().is_ok());
        running.await?;
        Ok(Self { hold, worker })
    }

    async fn release(self) -> TestResult {
        self.hold.send(())?;
        assert!(self.worker.await?, "blocking worker was not held");
        Ok(())
    }
}

async fn replies(client: &mut Client, count: usize) -> TestResult<BTreeMap<String, Value>> {
    let mut replies = BTreeMap::new();
    while replies.len() < count {
        let reply = client.expect_next().await?;
        let id = at(&reply, "/id").as_str().ok_or("reply id")?.to_owned();
        replies.insert(id, reply);
    }
    Ok(replies)
}

/// One attempt; returns the cancellation outcome after checking that no effect began.
async fn pre_accept(fixture: &Fixture, client: &mut Client, name: &str) -> TestResult<Value> {
    // Given a reserved request whose durable accept is queued behind the held worker.
    let blocker = Blocker::occupy().await?;
    let op = op_id();
    client.send(&set("set", &op, (name, "v"))).await?;
    client.send(&set("probe", &op, (name, "other"))).await?;
    let loser = client.expect_next().await?;
    assert_eq!(error_kind(&loser), "op_reused", "{loser}");
    let winner = if at(&loser, "/id") == "set" {
        "probe"
    } else {
        "set"
    };
    // When op.cancel arrives before the accept can commit.
    client
        .send(&request("cancel", "op.cancel", &json!({"op_id": op})))
        .await?;
    client.send(&set("after", &op, (name, "third"))).await?;
    assert_eq!(error_kind(&client.expect_next().await?), "op_reused");
    blocker.release().await?;
    let settled = replies(client, 2).await?;
    // Then both replies name the cancelled operation and the handler never ran.
    let cancel = settled.get("cancel").ok_or("cancel reply")?;
    let original = settled.get(winner).ok_or("request reply")?;
    assert_eq!(at(cancel, "/result/op_id"), &json!(op), "{cancel}");
    assert_eq!(at(cancel, "/result/state"), "cancelled", "{cancel}");
    assert_eq!(error_kind(original), "cancelled", "{original}");
    assert_eq!(at(original, "/error/data/op_id"), &json!(op));
    assert!(!fixture.effect(name).exists());
    let outcome = at(cancel, "/result/outcome").clone();
    if outcome == "cancel_pending" {
        assert!(!fixture.gate.entered(name)?, "handler ran after cancel");
    }
    client
        .send(&request("inspect", "op.get", &json!({"op_id": op})))
        .await?;
    assert_eq!(
        at(&client.expect_next().await?, "/result/state"),
        "cancelled"
    );
    Ok(outcome)
}

/// Pre-accept cancellation is answered `cancel_pending` and the request settles `cancelled`.
pub(crate) fn happy() -> TestResult {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let fixture = Fixture::new()?;
        let server = fixture
            .socket
            .serve(1, std::sync::Arc::clone(&fixture.dispatch))
            .await?;
        let mut client = fixture.client(&fixture.socket.bearer).await?;
        let mut outcomes = Vec::new();
        // A reordered attempt still checks every invariant; it only cannot count as pending.
        for attempt in 0..20 {
            let outcome = pre_accept(&fixture, &mut client, &format!("held-{attempt}")).await?;
            outcomes.push(outcome.clone());
            if outcome == "cancel_pending" {
                break;
            }
        }
        writeln!(
            std::io::stdout(),
            "outcomes={}",
            Value::from(outcomes.clone())
        )?;
        assert_eq!(outcomes.last(), Some(&json!("cancel_pending")));
        drop(client);
        server.shutdown().await?;
        fixture.close().await
    })
}

#[derive(Debug, Default)]
struct Tally {
    pending: usize,
    before_commit: usize,
    unknown: usize,
}

impl std::fmt::Display for Tally {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cancel_pending={} cancelled_before_commit={} not_found={}",
            self.pending, self.before_commit, self.unknown
        )
    }
}

async fn race_once(fixture: &Fixture, client: &mut Client, tally: &mut Tally) -> TestResult {
    let name = format!("race-{}", op_id());
    let op = op_id();
    client.send(&set("set", &op, (&name, "v"))).await?;
    client
        .send(&request("cancel", "op.cancel", &json!({"op_id": op})))
        .await?;
    let first = client.expect_next().await?;
    if at(&first, "/id") == "cancel" && error_kind(&first) == "not_found" {
        // The cancel was dispatched before the request was: an unknown identity.
        fixture.gate.release.notify_one();
    }
    let second = client.expect_next().await?;
    let (cancel, original) = if at(&first, "/id") == "cancel" {
        (first, second)
    } else {
        (second, first)
    };
    match at(&cancel, "/result/outcome").as_str() {
        Some("cancel_pending") => {
            tally.pending = tally.pending.saturating_add(1);
            assert!(!fixture.gate.entered(&name)?, "handler ran: {cancel}");
        }
        Some("cancelled_before_commit") => {
            tally.before_commit = tally.before_commit.saturating_add(1);
        }
        _ if error_kind(&cancel) == "not_found" => {
            tally.unknown = tally.unknown.saturating_add(1);
            assert_eq!(at(&original, "/result/value"), "v", "{original}");
            assert_eq!(std::fs::read(fixture.effect(&name))?, b"\"v\"");
            return Ok(());
        }
        _ => return Err(format!("unexpected cancellation {cancel}").into()),
    }
    assert_eq!(error_kind(&original), "cancelled", "{original}");
    assert!(!fixture.effect(&name).exists(), "effect after {cancel}");
    Ok(())
}

async fn race_200() -> TestResult<Tally> {
    // Given one connection that pipelines a mutation and its cancellation.
    let fixture = Fixture::new()?;
    let server = fixture
        .socket
        .serve(1, std::sync::Arc::clone(&fixture.dispatch))
        .await?;
    let mut client = fixture.client(&fixture.socket.bearer).await?;
    let mut tally = Tally::default();
    // When cancellation races the accept 200 times.
    for _ in 0..200 {
        race_once(&fixture, &mut client, &mut tally).await?;
    }
    writeln!(std::io::stdout(), "tally {tally}")?;
    drop(client);
    server.shutdown().await?;
    fixture.close().await?;
    // Then every interleaving held its invariant.
    assert_eq!(
        tally
            .pending
            .saturating_add(tally.before_commit)
            .saturating_add(tally.unknown),
        200
    );
    Ok(tally)
}

/// One scheduler thread dispatches both pipelined requests before the accept's worker runs.
#[tokio::test]
async fn cancel_wins_the_race_against_the_durable_accept() -> TestResult {
    let tally = race_200().await?;
    assert!(
        tally.pending >= 1,
        "cancel never preceded the accept: {tally}"
    );
    Ok(())
}

/// Parallel workers also interleave the cancel before the request is even reserved.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_parallel_interleaving_keeps_cancel_truthful() -> TestResult {
    race_200().await.map(drop)
}
