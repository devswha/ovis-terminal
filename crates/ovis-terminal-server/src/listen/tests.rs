//! Peer accounts collected at accept decide admission through the real listener path.
#![cfg(unix)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use ovis_terminal_authz::{
    Authority, AuthorityConfig, OwnerSpec, PrincipalId, RuntimeBinding, UnixUid,
};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, RuntimeId, RuntimeIdentity, ServerBootId,
};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use super::{ServeReport, ServerConfig, UnixEndpoint, serve_unix};
use crate::conn::{Call, RequestHandler};

static NEXT_DIR: AtomicU32 = AtomicU32::new(0);

#[derive(Default)]
struct Counting(AtomicU64);

impl RequestHandler for Counting {
    fn methods(&self) -> Vec<&'static str> {
        vec!["doc.get"]
    }

    fn call(&self, _call: Call) -> impl Future<Output = Result<Value, BoundaryError>> + Send {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::future::ready(Ok(json!({})))
    }
}

/// A peer whose `SO_PEERCRED` uid differs from the real one in its lowest bit.
fn foreign_peer(stream: &UnixStream) -> std::io::Result<UnixUid> {
    stream.peer_cred().map(|peer| UnixUid::new(peer.uid() ^ 1))
}

struct Outcome {
    hello: Value,
    report: ServeReport,
    calls: u64,
}

/// Serves one hello with a valid bearer through `bind`, the accept loop and the
/// connection core, with peer accounts from `source` when one is given.
async fn hello_from(source: Option<fn(&UnixStream) -> std::io::Result<UnixUid>>) -> Outcome {
    let unique = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("ovt13u-{}-{unique}", std::process::id()));
    let path: PathBuf = root.join("run").join("s.sock");
    let machine = "machine:0191f1a2-0000-7000-8000-0000000000e2"
        .parse()
        .unwrap();
    let host = HostIdentity::new(machine, HostBootId::try_from(1).unwrap());
    let runtime = RuntimeIdentity::new(
        host,
        RuntimeId::try_from(3).unwrap(),
        ServerBootId::try_from(1).unwrap(),
    );
    let binding = RuntimeBinding::new(machine, runtime.runtime_id());
    let owner = OwnerSpec::new(
        PrincipalId::parse("owner").unwrap(),
        TimestampMs::new(u64::MAX),
    );
    let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(binding, owner)).unwrap();
    let mut endpoint = UnixEndpoint::bind(&path).await.unwrap();
    if let Some(source) = source {
        endpoint = endpoint.with_peer_uid(source);
    }
    let handler = Arc::new(Counting::default());
    let config = ServerConfig::new(Arc::new(authority), runtime);
    let server = serve_unix(endpoint, config, Arc::clone(&handler)).unwrap();

    let mut stream = UnixStream::connect(&path).await.unwrap();
    let hello = json!({
        "jsonrpc": "2.0", "id": "h", "method": "hello",
        "params": { "generation": "1", "client": { "name": "t", "version": "1" },
                    "credential": issued.bearer() },
    });
    stream
        .write_all(format!("{hello}\n").as_bytes())
        .await
        .unwrap();
    let mut reply = String::new();
    let mut reader = BufReader::new(stream);
    let read = tokio::time::timeout(Duration::from_secs(10), reader.read_line(&mut reply));
    read.await.unwrap().unwrap();
    drop(reader);
    let report = server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    Outcome {
        hello: serde_json::from_str(&reply).unwrap(),
        report,
        calls: handler.0.load(Ordering::SeqCst),
    }
}

#[tokio::test]
async fn a_peer_whose_account_is_not_the_owner_is_refused_at_hello() {
    let refused = hello_from(Some(foreign_peer)).await;
    let kind = refused.hello.pointer("/error/data/kind");
    assert_eq!(kind, Some(&json!("peer_rejected")));
    assert!(
        refused.hello.get("result").is_none(),
        "no identity was exposed"
    );
    assert_eq!(refused.calls, 0);
    assert_eq!(refused.report.failures().get("peer_rejected"), Some(&1));
}

#[tokio::test]
async fn the_owner_account_read_from_the_socket_is_admitted() {
    let admitted = hello_from(None).await;
    assert!(
        admitted.hello.pointer("/result/server_boot_id").is_some(),
        "{}",
        admitted.hello
    );
    assert!(
        admitted.report.failures().is_empty(),
        "{:?}",
        admitted.report.failures()
    );
}
