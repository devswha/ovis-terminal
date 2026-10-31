//! Proves a read-side EOF keeps failure accounting live while requests drain.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use ovis_terminal_authz::{
    Authority, AuthorityConfig, EndpointPolicy, FileEvidence, LocalAccount, OwnerSpec,
    PeerEvidence, PrincipalId, RuntimeBinding, UnixSocketEvidence, UnixUid,
};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, RuntimeId, RuntimeIdentity, ServerBootId,
};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, DuplexStream,
    ReadBuf,
};
use tokio::sync::{Notify, mpsc, oneshot};

use crate::conn::{Call, Connections, Notifier, RequestHandler};

const WAIT: Duration = Duration::from_secs(20);

struct PendingRequest {
    retained: mpsc::Sender<Notifier>,
    release: Arc<Notify>,
}

impl RequestHandler for PendingRequest {
    fn methods(&self) -> Vec<&'static str> {
        vec!["doc.get"]
    }

    async fn call(&self, call: Call) -> Result<Value, BoundaryError> {
        self.retained.send(call.notifier).await.unwrap();
        self.release.notified().await;
        Ok(json!({}))
    }
}

struct ObservedEof {
    stream: DuplexStream,
    observed: Option<oneshot::Sender<()>>,
}

impl AsyncRead for ObservedEof {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buffer.filled().len();
        let result = Pin::new(&mut this.stream).poll_read(context, buffer);
        if matches!(&result, Poll::Ready(Ok(())))
            && buffer.filled().len() == before
            && let Some(signal) = this.observed.take()
        {
            signal.send(()).unwrap();
        }
        result
    }
}

impl AsyncWrite for ObservedEof {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(context, buffer)
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(context)
    }
}

#[tokio::test(flavor = "current_thread")]
async fn overflow_while_draining_after_read_eof_is_not_success() {
    tokio::time::timeout(WAIT, async {
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
        let (authority, issued) =
            Authority::bootstrap(AuthorityConfig::new(binding, owner)).unwrap();
        let (notifiers, mut retained) = mpsc::channel(1);
        let release = Arc::new(Notify::new());
        let handler = Arc::new(PendingRequest {
            retained: notifiers,
            release: Arc::clone(&release),
        });
        let connections = Connections::new(Arc::new(authority), runtime, handler).unwrap();
        let uid = UnixUid::new(501);
        let evidence = UnixSocketEvidence::new(
            FileEvidence::new(uid, 0o600),
            FileEvidence::new(uid, 0o700),
            uid,
        );
        let policy = EndpointPolicy::new(LocalAccount::Unix(uid));
        let admission = policy.admit(&PeerEvidence::UnixSocket(evidence));
        let (client, stream) = tokio::io::duplex(8192);
        let (observed, eof) = oneshot::channel();
        let stream = ObservedEof {
            stream,
            observed: Some(observed),
        };
        let driver = tokio::spawn(connections.serve(stream, admission));
        let mut client = BufReader::new(client);
        let hello = json!({
            "jsonrpc": "2.0", "id": "h", "method": "hello",
            "params": {
                "generation": "1",
                "client": {"name": "gate", "version": "1"},
                "credential": issued.bearer()
            }
        });
        client
            .get_mut()
            .write_all(format!("{hello}\n").as_bytes())
            .await
            .unwrap();
        let mut line = String::new();
        client.read_line(&mut line).await.unwrap();
        let hello: Value = serde_json::from_str(&line).unwrap();
        assert!(hello.get("result").is_some(), "{hello}");
        client
            .get_mut()
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":\"r\",\"method\":\"doc.get\",\"params\":{}}\n")
            .await
            .unwrap();
        let notifier = retained.recv().await.unwrap();

        client.get_mut().shutdown().await.unwrap();
        eof.await.unwrap();
        assert!(!driver.is_finished());
        let refused = notifier
            .notify("event", json!({"bytes": "x".repeat(64 * 1024 * 1024)}))
            .await
            .unwrap_err();
        assert_eq!(refused.kind(), "backpressure");
        println!("HALF_CLOSE_NOTIFY={refused:?}");
        release.notify_one();
        let mut wire = String::new();
        client.read_to_string(&mut wire).await.unwrap();
        println!("HALF_CLOSE_WIRE={wire:?}");
        let outcome = driver.await.unwrap();
        println!("HALF_CLOSE_OUTCOME={outcome:?}");
        let closing_seen = wire.lines().any(|line| {
            let message: Value = serde_json::from_str(line).unwrap();
            message.pointer("/params/reason") == Some(&json!("backpressure"))
        });
        assert!(
            closing_seen,
            "read-side EOF must not suppress the backpressure closing"
        );
        let error = outcome.expect_err("overflow during request drain must be reported");
        assert_eq!(error.kind(), "backpressure");
    })
    .await
    .unwrap();
}
