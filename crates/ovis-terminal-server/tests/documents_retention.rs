//! `doc.get` and `doc.set` through the real socket, dispatcher, ledger and SQLite store
//! (plan todo 50). `happy` and `failure` are the QA entry points named by the plan.
#![cfg(unix)]

#[path = "documents_retention/large.rs"]
mod large;
#[path = "documents_retention/scope.rs"]
mod scope;
#[path = "../src/dispatch/qa/socket.rs"]
pub mod socket;

use std::sync::Arc;

use ovis_terminal_core::docs::DocumentsRetentionEvent;
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::OperationProducer;
use ovis_terminal_server::dispatch::Dispatcher;
use ovis_terminal_server::dispatch::lanes::documents_retention;
use ovis_terminal_server::listen::ServerHandle;
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};
use socket::{Client, at, error_kind, request, runtime};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Core = Coordinator<DocumentsRetentionEvent, OperationProducer>;

struct Served {
    socket: socket::Fixture,
    core: Arc<Core>,
    dispatch: Arc<Dispatcher<DocumentsRetentionEvent, OperationProducer>>,
    server: ServerHandle,
}

impl Served {
    async fn start() -> TestResult<Self> {
        let socket = socket::Fixture::new()?;
        let identity = runtime(1)?;
        let store = Store::open(&socket.path("db"), StoreOptions::default())?;
        let producer = OperationProducer::new(identity.runtime_id());
        let core = Arc::new(Coordinator::new(store, identity, producer));
        let mut dispatch =
            Dispatcher::new(Arc::clone(&socket.authority), identity, Arc::clone(&core));
        documents_retention::register(&mut dispatch, identity)?;
        let dispatch = Arc::new(dispatch);
        let server = socket.serve(1, Arc::clone(&dispatch)).await?;
        Ok(Self {
            socket,
            core,
            dispatch,
            server,
        })
    }

    async fn client(&self) -> TestResult<Client> {
        let mut client = self.socket.connect().await?;
        client.hello("1", &self.socket.bearer).await?;
        Ok(client)
    }

    async fn close(self) -> TestResult {
        self.server.shutdown().await?;
        self.dispatch.shutdown().await?;
        drop(self.dispatch);
        let core = Arc::try_unwrap(self.core).map_err(|_| "core still shared")?;
        tokio::task::spawn_blocking(move || core.close()).await??;
        Ok(())
    }
}

/// A `UUIDv7` minted `age_ms` before the wall clock.
fn op(age_ms: u64) -> TestResult<String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?;
    let at = u64::try_from(now.as_millis())?
        .checked_sub(age_ms)
        .ok_or("before epoch")?;
    let stamp = uuid::Timestamp::from_unix(
        uuid::NoContext,
        at / 1000,
        u32::try_from(at % 1000)?
            .checked_mul(1_000_000)
            .ok_or("nanos")?,
    );
    Ok(uuid::Uuid::new_v7(stamp).to_string())
}

fn set(op_id: &str, mut params: Value) -> Value {
    if let Some(object) = params.as_object_mut() {
        object.insert("op_id".to_owned(), json!(op_id));
    }
    request(op_id, "doc.set", &params)
}

async fn call(client: &mut Client, message: &Value) -> TestResult<Value> {
    client.send(message).await?;
    client.expect_next().await
}

async fn get(client: &mut Client, name: &str) -> TestResult<Value> {
    let reply = call(client, &request("get", "doc.get", &json!({"name": name}))).await?;
    Ok(at(&reply, "/result").clone())
}

#[tokio::test]
async fn happy() -> TestResult {
    // Given: two connections and a subscriber installed before either writes.
    let served = Served::start().await?;
    let observer = served.core.subscribe(None)?;
    let (mut first, mut second) = (served.client().await?, served.client().await?);

    // When: both send doc.set at version 0 before either reads its reply.
    first
        .send(&set(
            &op(0)?,
            json!({"name": "shared", "version": "0", "value": {"x": 1}}),
        ))
        .await?;
    second
        .send(&set(
            &op(0)?,
            json!({"name": "shared", "version": "0", "value": {"x": 2}}),
        ))
        .await?;
    let replies = [first.expect_next().await?, second.expect_next().await?];

    // Then: exactly one commits version 1 with its event; the other is a typed conflict.
    let winner = replies
        .iter()
        .position(|reply| reply.get("result").is_some())
        .ok_or("no commit")?;
    let loser = replies.get(1 - winner).ok_or("loser")?;
    let committed = replies.get(winner).ok_or("winner")?;
    assert_eq!(at(committed, "/result/version"), "1", "{replies:?}");
    assert_eq!(error_kind(loser), "conflict", "{replies:?}");
    assert_eq!(at(loser, "/error/data/op_id"), at(loser, "/id"));
    let changes: Vec<_> = std::iter::from_fn(|| observer.delivery.try_recv().ok())
        .filter(|event| event.event_type == "doc.changed")
        .collect();
    assert_eq!(changes.len(), 1);
    assert_eq!(
        at(committed, "/result/seq"),
        &json!(
            changes
                .first()
                .ok_or("change")?
                .cursor
                .seq
                .get()
                .to_string()
        )
    );
    let document = get(&mut first, "shared").await?;
    assert_eq!(
        document,
        json!({"name": "shared", "version": "1", "value": {"x": winner + 1}})
    );
    drop((first, second));
    served.close().await
}

#[tokio::test]
async fn failure() -> TestResult {
    // Given: "shared" committed at version 1.
    let served = Served::start().await?;
    let mut client = served.client().await?;
    call(
        &mut client,
        &set(
            &op(0)?,
            json!({"name": "shared", "version": "0", "value": {"x": 1}}),
        ),
    )
    .await?;

    // When: the injected stale write arrives, and is retried with the same op id.
    let stale = op(0)?;
    let injected = json!({"name": "shared", "version": "0", "value": {"x": 2}});
    let rejected = call(&mut client, &set(&stale, injected.clone())).await?;
    let retried = call(&mut client, &set(&stale, injected)).await?;

    // Then: both are the retained conflict, and the document is still version 1.
    assert_eq!(
        (error_kind(&rejected), error_kind(&retried)),
        ("conflict", "conflict")
    );
    assert_eq!(at(&rejected, "/error/data/op_id"), &json!(stale));
    assert_eq!(
        get(&mut client, "shared").await?,
        json!({"name": "shared", "version": "1", "value": {"x": 1}})
    );
    drop(client);
    served.close().await
}

#[tokio::test]
async fn adversarial_inputs_fail_typed_and_leave_the_version_unchanged() -> TestResult {
    // Given: "shared" committed at version 1.
    let served = Served::start().await?;
    let mut client = served.client().await?;
    call(
        &mut client,
        &set(
            &op(0)?,
            json!({"name": "shared", "version": "0", "value": "small"}),
        ),
    )
    .await?;
    // A 1 MiB + 1 byte value. Over the socket the 256 KiB canonical-params bound of the
    // operation ledger refuses it before admission, so the reply carries no op_id; the
    // inclusive 1 MiB document bound itself is asserted by the core bounds tests.
    let oversized = Value::String("x".repeat(1_048_575));
    let refused = call(
        &mut client,
        &set(
            &op(0)?,
            json!({"name": "shared", "version": "1", "value": oversized}),
        ),
    )
    .await?;
    assert_eq!(error_kind(&refused), "too_large", "{refused}");
    assert_eq!(at(&refused, "/error/data/op_id"), &Value::Null, "{refused}");

    // When: an expired op id, a numeric version, a bad name and a foreign machine are each
    // presented.
    let cases = [
        (
            set(
                &op(86_400_000 + 60_000)?,
                json!({"name": "shared", "version": "1", "value": 1}),
            ),
            "op_expired",
        ),
        (
            set(&op(0)?, json!({"name": "shared", "version": 1, "value": 1})),
            "invalid_params",
        ),
        (
            set(&op(0)?, json!({"name": "a/b", "version": "0", "value": 1})),
            "invalid_name",
        ),
        (
            set(
                &op(0)?,
                json!({"name": "shared", "version": "1", "value": 1, "machine": {"machine_id": "machine:other"}}),
            ),
            "not_found",
        ),
    ];
    for (message, kind) in cases {
        let reply = call(&mut client, &message).await?;

        // Then: each is its typed error and nothing was written.
        assert_eq!(error_kind(&reply), kind, "{reply}");
    }
    assert_eq!(
        get(&mut client, "shared").await?,
        json!({"name": "shared", "version": "1", "value": "small"})
    );
    drop(client);
    served.close().await
}
