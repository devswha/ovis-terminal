//! A request that triggered a planned restart is answered before the connection closes:
//! its handler returned, so its reply is written ahead of `$/closing`, while a request
//! still running is cancelled without a reply.

use std::sync::Arc;

use ovis_terminal_authz::{Authority, AuthorityConfig, OwnerSpec, PrincipalId, RuntimeBinding};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, RuntimeId, RuntimeIdentity, ServerBootId,
};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::watch;

use super::{
    Connections, DEFAULT_SHUTDOWN_GRACE, ServeReport, ServerHandle, Serving, UnixEndpoint,
    accept_loop,
};
use crate::conn::{Call, Ending, RequestHandler};

const WAIT: std::time::Duration = std::time::Duration::from_secs(10);
const ROUNDS: usize = 20;

/// `server.restart` asks for the planned restart and then answers, as the assembled
/// `server.restart` and `update.activate` do; `terminal.wait_output` never finishes.
struct Restarting(watch::Sender<Option<Ending>>);

impl RequestHandler for Restarting {
    fn methods(&self) -> Vec<&'static str> {
        vec!["server.restart", "terminal.wait_output"]
    }

    async fn call(&self, call: Call) -> Result<Value, BoundaryError> {
        if call.request.method() == "terminal.wait_output" {
            return std::future::pending().await;
        }
        self.0.send_replace(Some(Ending::Restart));
        Ok(json!({"restarted": true}))
    }
}

struct SocketRoot(std::path::PathBuf);

impl Drop for SocketRoot {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

async fn server(round: usize) -> (SocketRoot, ServerHandle, String) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("ovt31r-{}-{round}-{nanos:x}", std::process::id());
    let root = SocketRoot(std::env::temp_dir().join(name));
    std::os::unix::fs::DirBuilderExt::mode(&mut std::fs::DirBuilder::new(), 0o700)
        .create(&root.0)
        .unwrap();
    let machine = "machine:0191f1a2-0000-7000-8000-000000000031"
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
    let (signal, shutdown) = watch::channel(None);
    let handler = Arc::new(Restarting(signal.clone()));
    let connections = Connections::new(Arc::new(authority), runtime, handler).unwrap();
    let endpoint = UnixEndpoint::bind(&root.0.join("s.sock")).await.unwrap();
    let (progress, observed) = watch::channel(ServeReport::default());
    let serving = Serving {
        connections,
        shutdown,
        progress,
        max_connections: 1,
        grace: DEFAULT_SHUTDOWN_GRACE,
        gate: None,
    };
    let task = tokio::spawn(accept_loop(endpoint, serving));
    let handle = ServerHandle {
        signal,
        progress: observed,
        task,
    };
    (root, handle, issued.bearer().to_owned())
}

async fn line(client: &mut BufReader<UnixStream>) -> Option<Value> {
    let mut line = String::new();
    let read = tokio::time::timeout(WAIT, client.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    (read > 0).then(|| serde_json::from_str(&line).unwrap())
}

#[tokio::test]
async fn the_reply_of_the_call_that_restarted_precedes_closing_and_a_running_call_gets_none() {
    for round in 0..ROUNDS {
        // Given: a connected client with a request that never finishes in flight.
        let (root, server, bearer) = server(round).await;
        let stream = UnixStream::connect(root.0.join("s.sock")).await.unwrap();
        let mut client = BufReader::new(stream);
        let hello = json!({
            "jsonrpc": "2.0", "id": "h", "method": "hello",
            "params": {"generation": "1", "client": {"name": "t31", "version": "1"},
                       "credential": bearer}
        });
        let hold =
            json!({"jsonrpc": "2.0", "id": "hold", "method": "terminal.wait_output", "params": {}});
        let restart =
            json!({"jsonrpc": "2.0", "id": "restart", "method": "server.restart", "params": {}});
        client
            .get_mut()
            .write_all(format!("{hello}\n{hold}\n{restart}\n").as_bytes())
            .await
            .unwrap();
        let greeted = line(&mut client).await.unwrap();
        assert!(greeted.get("result").is_some(), "{greeted}");

        // When: the call asks for the restart and returns.
        let mut records = Vec::new();
        while let Some(record) = line(&mut client).await {
            records.push(record);
        }

        // Then: its reply comes first, then `$/closing{server_restart}`, then end of
        // stream; the request still running is cancelled without a reply.
        let [reply, closing] = records.as_slice() else {
            panic!("round {round}: expected a reply then $/closing, got {records:?}");
        };
        assert_eq!(reply.get("id"), Some(&json!("restart")), "{records:?}");
        assert_eq!(
            reply.pointer("/result/restarted"),
            Some(&json!(true)),
            "{records:?}"
        );
        assert_eq!(closing.get("method"), Some(&json!("$/closing")));
        assert_eq!(
            closing.pointer("/params/reason"),
            Some(&json!("server_restart"))
        );
        server.restart().await.unwrap();
    }
}
