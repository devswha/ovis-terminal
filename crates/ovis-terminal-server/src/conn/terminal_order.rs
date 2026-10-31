//! Forces a transport failure to precede an overflow published during teardown.

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
    AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, DuplexStream, ReadBuf,
};
use tokio::sync::{mpsc, oneshot};

use crate::conn::{Call, Connections, Notifier, RequestHandler};

const WAIT: Duration = Duration::from_secs(20);

struct RetainNotifier(mpsc::Sender<Notifier>);

impl RequestHandler for RetainNotifier {
    fn methods(&self) -> Vec<&'static str> {
        vec!["doc.get"]
    }

    async fn call(&self, call: Call) -> Result<Value, BoundaryError> {
        self.0.send(call.notifier).await.unwrap();
        Ok(json!({}))
    }
}

struct FaultingStream {
    stream: DuplexStream,
    fail_read: oneshot::Receiver<()>,
    finish_shutdown: oneshot::Receiver<()>,
}

impl AsyncRead for FaultingStream {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if Pin::new(&mut this.fail_read).poll(context).is_ready() {
            return Poll::Ready(Err(io::Error::other("transport failed before overflow")));
        }
        Pin::new(&mut this.stream).poll_read(context, buffer)
    }
}

impl AsyncWrite for FaultingStream {
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
        let this = self.get_mut();
        match Pin::new(&mut this.finish_shutdown).poll(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(_) => Pin::new(&mut this.stream).poll_shutdown(context),
        }
    }
}

#[tokio::test]
async fn io_before_overflow_keeps_its_terminal_cause() {
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
        let handler = Arc::new(RetainNotifier(notifiers));
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
        let (fail_read, read_failure) = oneshot::channel();
        let (finish_shutdown, shutdown_release) = oneshot::channel();
        let stream = FaultingStream {
            stream,
            fail_read: read_failure,
            finish_shutdown: shutdown_release,
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
        line.clear();
        client.read_line(&mut line).await.unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert!(reply.get("result").is_some(), "{reply}");

        fail_read.send(()).unwrap();
        line.clear();
        client.read_line(&mut line).await.unwrap();
        let closing: Value = serde_json::from_str(&line).unwrap();
        println!("EARLIER_IO_CLOSE={closing}");
        assert_eq!(
            closing.pointer("/params/reason"),
            Some(&json!("connection_io"))
        );
        assert!(!driver.is_finished());

        let overflow = notifier
            .notify("event", json!({"bytes": "x".repeat(64 * 1024 * 1024)}))
            .await
            .unwrap_err();
        assert_eq!(overflow.kind(), "backpressure");
        finish_shutdown.send(()).unwrap();
        let error = driver.await.unwrap().unwrap_err();
        println!("EARLIER_IO_FINAL_ERROR={error:?}");
        assert_eq!(
            error.kind(),
            "connection_io",
            "an earlier transport failure must not be replaced by a later overflow"
        );
    })
    .await
    .unwrap();
}
