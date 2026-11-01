//! `server.status` and `server.metrics` through the real listener, connection core,
//! dispatcher, authority and SQLite store, with [`Measured`] in front of them.
#![cfg(unix)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ovis_terminal_authz::{Authority, AuthorityConfig, OwnerSpec, PrincipalId, RuntimeBinding};
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::health::{Health, RecoveryLedger};
use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
use ovis_terminal_core::recovery::{Finding, Incidents};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, RuntimeId, RuntimeIdentity, ServerBootId,
};
use ovis_terminal_protocol::domain::recovery::IncidentKind;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::ids;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{Notify, watch};

use super::{Measured, register};
use crate::conn::{Call, RequestHandler};
use crate::dispatch::Dispatcher;
use crate::listen::{ServerConfig, ServerHandle, UnixEndpoint, serve_unix};

type Dispatch = Dispatcher<OperationEvent, OperationProducer>;

const BOUND: Duration = Duration::from_secs(10);
const HELD: &str = "doc.set";

/// The dispatcher plus one catalog method whose call stays pending until released.
struct WithHeld {
    dispatch: Arc<Dispatch>,
    entered: watch::Sender<bool>,
    release: Notify,
}

impl RequestHandler for WithHeld {
    fn methods(&self) -> Vec<&'static str> {
        let mut methods = self.dispatch.methods();
        methods.push(HELD);
        methods
    }

    async fn call(&self, call: Call) -> Result<Value, BoundaryError> {
        if call.request.method() != HELD {
            return self.dispatch.call(call).await;
        }
        let released = self.release.notified();
        self.entered.send_replace(true);
        released.await;
        Ok(json!({}))
    }
}

struct Rig {
    root: PathBuf,
    health: Arc<Health>,
    handler: Arc<Measured<WithHeld>>,
    server: ServerHandle,
    bearer: String,
}

fn runtime() -> RuntimeIdentity {
    let machine = "machine:0191f1a2-0000-7000-8000-000000000059"
        .parse()
        .unwrap();
    RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1).unwrap()),
        RuntimeId::try_from(59).unwrap(),
        ServerBootId::try_from(4).unwrap(),
    )
}

async fn rig() -> Rig {
    let root = std::env::temp_dir().join(format!("ovt59-{}", uuid::Uuid::now_v7().simple()));
    std::os::unix::fs::DirBuilderExt::mode(&mut std::fs::DirBuilder::new(), 0o700)
        .create(&root)
        .unwrap();
    let runtime = runtime();
    let binding = RuntimeBinding::new(runtime.machine_id(), runtime.runtime_id());
    let owner = OwnerSpec::new(
        PrincipalId::parse("owner").unwrap(),
        TimestampMs::new(u64::MAX),
    );
    let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(binding, owner)).unwrap();
    let authority = Arc::new(authority);
    let store = Store::open(&root.join("state.db"), StoreOptions::default()).unwrap();
    let core = Arc::new(Coordinator::new(
        store,
        runtime,
        OperationProducer::new(runtime.runtime_id()),
    ));
    let boot = ids::ServerBootId::new(runtime.server_boot_id().to_string());
    let ledger =
        RecoveryLedger::open(Incidents::new(&root), (boot, TimestampMs::new(1)), None).unwrap();
    let health = Arc::new(Health::new(runtime, ledger));
    let mut dispatch = Dispatch::new(Arc::clone(&authority), runtime, core);
    register(&mut dispatch, &health, runtime).unwrap();
    let held = WithHeld {
        dispatch: Arc::new(dispatch),
        entered: watch::channel(false).0,
        release: Notify::new(),
    };
    let handler = Arc::new(Measured::new(Arc::clone(&health), Arc::new(held)));
    let endpoint = UnixEndpoint::bind(&root.join("s.sock")).await.unwrap();
    let config = ServerConfig::new(authority, runtime);
    let server = serve_unix(endpoint, config, Arc::clone(&handler)).unwrap();
    Rig {
        root,
        health,
        handler,
        server,
        bearer: issued.bearer().to_owned(),
    }
}

/// One connection that keeps replies which arrive before the one awaited.
struct Client {
    lines: Lines<BufReader<OwnedReadHalf>>,
    writer: OwnedWriteHalf,
    early: HashMap<String, Value>,
}

impl Client {
    async fn open(rig: &Rig) -> (Self, Value) {
        let (read, writer) = UnixStream::connect(rig.root.join("s.sock"))
            .await
            .unwrap()
            .into_split();
        let mut client = Self {
            lines: BufReader::new(read).lines(),
            writer,
            early: HashMap::new(),
        };
        let params = json!({"generation": "1", "client": {"name": "t59", "version": "1"}, "credential": rig.bearer});
        let hello = client.call(("hello", "hello"), params).await;
        (client, hello)
    }

    async fn send(&mut self, (id, method): (&str, &str), params: Value) {
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.writer
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
    }

    async fn reply(&mut self, id: &str) -> Value {
        loop {
            if let Some(reply) = self.early.remove(id) {
                return reply;
            }
            let line = tokio::time::timeout(BOUND, self.lines.next_line())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let reply: Value = serde_json::from_str(&line).unwrap();
            let key = at(&reply, "/id").as_str().unwrap_or_default().to_owned();
            self.early.insert(key, reply);
        }
    }

    async fn call(&mut self, (id, method): (&str, &str), params: Value) -> Value {
        self.send((id, method), params).await;
        self.reply(id).await
    }
}

async fn close(rig: Rig) {
    rig.server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&rig.root).unwrap();
}

fn at<'a>(value: &'a Value, path: &str) -> &'a Value {
    value.pointer(path).unwrap_or(&Value::Null)
}

fn latency<'a>(metrics: &'a Value, method: &str) -> Option<&'a Value> {
    at(metrics, "/result/rpc_latency")
        .as_array()
        .unwrap()
        .iter()
        .find(|row| at(row, "/method").as_str() == Some(method))
}

#[tokio::test]
async fn metrics_answer_while_a_call_hangs_and_name_the_current_boot() {
    let rig = rig().await;
    let (mut client, hello) = Client::open(&rig).await;
    let mut entered = rig.handler.inner.entered.subscribe();
    client
        .send(("held", HELD), json!({"op_id": "op-held"}))
        .await;
    entered.wait_for(|entered| *entered).await.unwrap();
    let during = client.call(("m1", "server.metrics"), json!({})).await;
    assert_eq!(
        at(&during, "/result/server_boot_id"),
        at(&hello, "/result/server_boot_id")
    );
    assert_eq!(
        at(&during, "/result/runtime_lifetime"),
        at(&hello, "/result/runtime_lifetime")
    );
    assert!(
        latency(&during, HELD).is_none(),
        "a pending call has no latency yet"
    );
    rig.handler.inner.release.notify_one();
    let held = client.reply("held").await;
    assert_eq!(at(&held, "/result"), &json!({}));
    let after = client.call(("m2", "server.metrics"), json!({})).await;
    let row = latency(&after, HELD).unwrap();
    assert_eq!(
        (at(row, "/calls").clone(), at(row, "/slowest_op").clone()),
        (json!("1"), json!("op-held"))
    );
    drop(client);
    close(rig).await;
}

#[tokio::test]
async fn reset_is_invalid_params_and_erases_no_counter_or_recovery_fact() {
    let rig = rig().await;
    let (mut client, _) = Client::open(&rig).await;
    let finding = Finding::new(IncidentKind::StoreCorrupted, Vec::new(), None);
    rig.health.recovery().record(finding).unwrap();
    let before = client.call(("m1", "server.metrics"), json!({})).await;
    assert_eq!(
        at(&before, "/result/recovery_state/corrupted"),
        &json!(true)
    );
    let reset = client
        .call(("m2", "server.metrics"), json!({"reset": true}))
        .await;
    assert_eq!(
        at(&reset, "/error/data/kind"),
        &json!("invalid_params"),
        "{reset}"
    );
    let after = client.call(("m3", "server.metrics"), json!({})).await;
    assert_eq!(
        at(&after, "/result/recovery_state"),
        at(&before, "/result/recovery_state")
    );
    let row = latency(&after, "server.metrics").unwrap();
    assert_eq!(
        (at(row, "/calls").clone(), at(row, "/failures").clone()),
        (json!("2"), json!("1"))
    );
    let status = client.call(("s1", "server.status"), json!({})).await;
    assert_eq!(at(&status, "/result/state"), &json!("running"));
    assert_eq!(
        at(&status, "/result/recovery_state/corrupted"),
        &json!(true)
    );
    let verbose = client
        .call(("s2", "server.status"), json!({"verbose": true}))
        .await;
    assert_eq!(at(&verbose, "/error/data/kind"), &json!("invalid_params"));
    drop(client);
    close(rig).await;
}
