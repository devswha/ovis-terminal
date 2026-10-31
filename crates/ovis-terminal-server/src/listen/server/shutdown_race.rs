//! Forces shutdown to race a published overflow while the connection cannot finish.

use std::future::poll_fn;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

use ovis_terminal_authz::{Authority, AuthorityConfig, OwnerSpec, PrincipalId, RuntimeBinding};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, RuntimeId, RuntimeIdentity, ServerBootId,
};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{Notify, watch};

use super::{
    Connections, DEFAULT_SHUTDOWN_GRACE, ServeReport, ServerHandle, Serving, UnixEndpoint,
    accept_loop,
};
use crate::conn::{Call, RequestHandler};

const WAIT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Debug, Default)]
struct Polling {
    paused: bool,
    waker: Option<Waker>,
}

/// Only the connection driver is gated; its real writer and requests still run.
#[derive(Debug, Default)]
pub(super) struct Gate {
    polling: Mutex<Polling>,
    pub(super) stopping: Notify,
}

impl Gate {
    fn pause(&self) {
        self.polling.lock().unwrap().paused = true;
    }

    fn release(&self) {
        let mut polling = self.polling.lock().unwrap();
        polling.paused = false;
        if let Some(waker) = polling.waker.take() {
            waker.wake();
        }
    }

    pub(super) async fn run<F: Future>(&self, connection: F) -> F::Output {
        tokio::pin!(connection);
        poll_fn(|cx| {
            let mut polling = self.polling.lock().unwrap();
            polling.waker = Some(cx.waker().clone());
            if polling.paused {
                return Poll::Pending;
            }
            drop(polling);
            connection.as_mut().poll(cx)
        })
        .await
    }
}

struct Overflowing(Arc<Gate>);

impl RequestHandler for Overflowing {
    fn methods(&self) -> Vec<&'static str> {
        vec!["doc.set"]
    }

    async fn call(&self, call: Call) -> Result<Value, BoundaryError> {
        self.0.pause();
        let refused = call
            .notifier
            .notify("event", json!({"bytes": "x".repeat(64 * 1024 * 1024)}))
            .await;
        assert_eq!(refused.unwrap_err().kind(), "backpressure");
        Ok(json!({}))
    }
}

struct SocketRoot(std::path::PathBuf);

impl Drop for SocketRoot {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

async fn server(gate: Arc<Gate>) -> (SocketRoot, ServerHandle, String) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        SocketRoot(std::env::temp_dir().join(format!("ovt75-{}-{nanos:x}", std::process::id())));
    std::os::unix::fs::DirBuilderExt::mode(&mut std::fs::DirBuilder::new(), 0o700)
        .create(&root.0)
        .unwrap();
    let machine = "machine:0191f1a2-0000-7000-8000-000000000075"
        .parse()
        .unwrap();
    let runtime = RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1).unwrap()),
        RuntimeId::try_from(1).unwrap(),
        ServerBootId::try_from(1).unwrap(),
    );
    let owner = OwnerSpec::new(
        PrincipalId::parse("owner").unwrap(),
        TimestampMs::new(u64::MAX),
    );
    let binding = RuntimeBinding::new(machine, runtime.runtime_id());
    let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(binding, owner)).unwrap();
    let connections = Connections::new(
        Arc::new(authority),
        runtime,
        Arc::new(Overflowing(Arc::clone(&gate))),
    )
    .unwrap();
    let endpoint = UnixEndpoint::bind(&root.0.join("s.sock")).await.unwrap();
    let (signal, shutdown) = watch::channel(false);
    let (progress, observed) = watch::channel(ServeReport::default());
    let serving = Serving {
        connections,
        shutdown,
        progress,
        max_connections: 1,
        grace: DEFAULT_SHUTDOWN_GRACE,
        gate: Some(gate),
    };
    let task = tokio::spawn(accept_loop(endpoint, serving));
    let handle = ServerHandle {
        signal,
        progress: observed,
        task,
    };
    (root, handle, issued.bearer().to_owned())
}

#[tokio::test]
async fn shutdown_joins_and_counts_an_overflow_before_the_driver_can_finish() {
    // Given a real accepted connection whose driver cannot observe its writer's overflow.
    let gate = Arc::new(Gate::default());
    let (root, server, bearer) = server(Arc::clone(&gate)).await;
    let stream = UnixStream::connect(root.0.join("s.sock")).await.unwrap();
    let mut client = BufReader::new(stream);
    let hello = json!({
        "jsonrpc": "2.0", "id": "h", "method": "hello",
        "params": {"generation": "1", "client": {"name": "t75", "version": "1"},
                   "credential": bearer}
    });
    client
        .get_mut()
        .write_all(format!("{hello}\n").as_bytes())
        .await
        .unwrap();
    let mut line = String::new();
    tokio::time::timeout(WAIT, client.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let hello: Value = serde_json::from_str(&line).unwrap();
    assert!(hello.get("result").is_some(), "{hello}");
    client
        .get_mut()
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":\"big\",\"method\":\"doc.set\",\"params\":{}}\n")
        .await
        .unwrap();
    line.clear();
    tokio::time::timeout(WAIT, client.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let closing: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(closing.get("method"), Some(&json!("$/closing")));
    assert_eq!(
        closing.pointer("/params/reason"),
        Some(&json!("backpressure"))
    );
    line.clear();
    assert_eq!(
        tokio::time::timeout(WAIT, client.read_line(&mut line))
            .await
            .unwrap()
            .unwrap(),
        0
    );

    // When shutdown has signalled the connection, it must still await the gated driver.
    let shutdown = tokio::spawn(server.shutdown());
    tokio::time::timeout(WAIT, gate.stopping.notified())
        .await
        .unwrap();
    assert!(
        !shutdown.is_finished(),
        "shutdown omitted the gated connection"
    );
    gate.release();
    let report = tokio::time::timeout(WAIT, shutdown)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    // Then its joined outcome retains the failure already observed by the peer.
    println!("FORCED_RACE_REPORT={report:?}");
    assert_eq!(report.accepted(), 1, "{report:?}");
    assert_eq!(
        report.failures().get("backpressure"),
        Some(&1),
        "{report:?}"
    );
    assert_eq!(report.aborted(), 0, "{report:?}");
    assert_eq!(report.panicked(), 0, "{report:?}");
}
