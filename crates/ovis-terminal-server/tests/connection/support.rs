//! A private socket directory, a bootstrapped authority and a real NDJSON/framed client.

use std::error::Error;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use ovis_terminal_authz::{Authority, AuthorityConfig, OwnerSpec, PrincipalId, RuntimeBinding};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RegistryIdentities, RuntimeId, RuntimeIdentity,
    ServerBootId,
};
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_protocol::wire::{
    Channel, ConnectionDecoder, FeedOutcome, Frame, FrameKind, WireMessage,
};
use ovis_terminal_server::conn::RequestHandler;
use ovis_terminal_server::listen::{ServerConfig, ServerHandle, UnixEndpoint, serve_unix};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

/// Result of a test step; any failure fails the test.
pub type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// Upper bound on any single wait for the server; never a pacing delay.
pub const WAIT: Duration = Duration::from_secs(10);
static NEXT_DIR: AtomicU32 = AtomicU32::new(0);

/// A private directory and the authority of one test runtime.
#[derive(Debug)]
pub struct Fixture {
    root: PathBuf,
    /// The authority every server of this fixture authenticates against.
    pub authority: Arc<Authority<RegistryIdentities>>,
    /// The owner credential's bearer text.
    pub bearer: String,
}

impl Fixture {
    /// A fixture whose owner credential never expires.
    ///
    /// # Errors
    ///
    /// Fails when the authority cannot be bootstrapped.
    pub fn new() -> TestResult<Self> {
        Self::expiring(TimestampMs::new(u64::MAX))
    }

    /// A fixture whose owner credential expires at `expires_at`.
    ///
    /// # Errors
    ///
    /// Fails when the authority cannot be bootstrapped.
    pub fn expiring(expires_at: TimestampMs) -> TestResult<Self> {
        let unique = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("ovt13-{}-{unique}", std::process::id()));
        let runtime = runtime(1)?;
        let binding = RuntimeBinding::new(runtime.machine_id(), runtime.runtime_id());
        let owner = OwnerSpec::new(PrincipalId::parse("owner")?, expires_at);
        let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(binding, owner))?;
        Ok(Self {
            root,
            authority: Arc::new(authority),
            bearer: issued.bearer().to_owned(),
        })
    }

    /// A socket path inside a directory this fixture owns and removes.
    pub fn path(&self, name: &str) -> PathBuf {
        self.root.join("run").join(name)
    }

    /// Binds the fixture socket and serves it with `handler` as control boot `boot`.
    ///
    /// # Errors
    ///
    /// Fails when binding or serving fails.
    pub async fn serve<H: RequestHandler>(
        &self,
        boot: u64,
        handler: Arc<H>,
    ) -> TestResult<ServerHandle> {
        let endpoint = UnixEndpoint::bind(&self.path("s.sock")).await?;
        let config = ServerConfig::new(Arc::clone(&self.authority), runtime(boot)?);
        Ok(serve_unix(endpoint, config, handler)?)
    }

    /// Connects a client to the fixture socket.
    ///
    /// # Errors
    ///
    /// Fails when the socket refuses the connection.
    pub async fn connect(&self) -> TestResult<Client> {
        Client::connect(&self.path("s.sock")).await
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.root) {
            let gone = error.kind() == std::io::ErrorKind::NotFound;
            assert!(
                gone || std::thread::panicking(),
                "fixture cleanup failed: {error}"
            );
        }
    }
}

/// Machine and runtime lifetime stay fixed; `boot` names the control-server process.
///
/// # Errors
///
/// Fails when an identity is not valid.
pub fn runtime(boot: u64) -> TestResult<RuntimeIdentity> {
    let machine: MachineId = "machine:0191f1a2-0000-7000-8000-000000000013".parse()?;
    let host = HostIdentity::new(machine, HostBootId::try_from(1)?);
    let runtime = RuntimeId::try_from(7)?;
    Ok(RuntimeIdentity::new(
        host,
        runtime,
        ServerBootId::try_from(boot)?,
    ))
}

/// A JSON-RPC request envelope.
pub fn request(id: &str, method: &str, params: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// The value at a JSON pointer, `null` when absent, so a missing field fails its assertion.
pub fn at<'a>(value: &'a Value, pointer: &str) -> &'a Value {
    value.pointer(pointer).unwrap_or(&Value::Null)
}

/// Error kind of a failure reply.
pub fn error_kind(reply: &Value) -> &str {
    at(reply, "/error/data/kind").as_str().unwrap_or_default()
}

fn hello_params(generation: &str, bearer: &str, framed: bool) -> Value {
    json!({
        "generation": generation,
        "client": { "name": "connection-test", "version": "1" },
        "framed": framed,
        "credential": bearer,
    })
}

/// A real client speaking NDJSON, then frames after a framed hello.
#[derive(Debug)]
pub struct Client {
    stream: UnixStream,
    decoder: ConnectionDecoder,
    pending: Vec<u8>,
    framed: bool,
}

impl Client {
    /// Connects to `path`.
    ///
    /// # Errors
    ///
    /// Fails when the socket refuses the connection.
    pub async fn connect(path: &Path) -> TestResult<Self> {
        Ok(Self {
            stream: UnixStream::connect(path).await?,
            decoder: ConnectionDecoder::new(),
            pending: Vec::new(),
            framed: false,
        })
    }

    /// Sends one JSON-RPC message in the current framing.
    ///
    /// # Errors
    ///
    /// Fails when encoding or writing fails.
    pub async fn send(&mut self, message: &Value) -> TestResult {
        let json = serde_json::to_vec(message)?;
        let bytes = if self.framed {
            Frame::new(Channel::CONTROL, FrameKind::Data, json)?.encode()?
        } else {
            [json, b"\n".to_vec()].concat()
        };
        self.send_raw(&bytes).await
    }

    /// Writes bytes verbatim.
    ///
    /// # Errors
    ///
    /// Fails when writing fails.
    pub async fn send_raw(&mut self, bytes: &[u8]) -> TestResult {
        Ok(self.stream.write_all(bytes).await?)
    }

    /// Sends a plain-mode hello and returns its reply.
    ///
    /// # Errors
    ///
    /// Fails when the server closes before replying.
    pub async fn hello(&mut self, generation: &str, bearer: &str) -> TestResult<Value> {
        self.negotiate(hello_params(generation, bearer, false))
            .await
    }

    /// Sends hello asking for frames; on success both directions switch to frames.
    ///
    /// # Errors
    ///
    /// Fails when the server closes before replying.
    pub async fn hello_framed(&mut self, generation: &str, bearer: &str) -> TestResult<Value> {
        let reply = self
            .negotiate(hello_params(generation, bearer, true))
            .await?;
        if reply.get("result").is_some() {
            self.decoder.upgrade_to_framed()?;
            self.framed = true;
        }
        Ok(reply)
    }

    async fn negotiate(&mut self, params: Value) -> TestResult<Value> {
        self.send(&request("hello", "hello", &params)).await?;
        self.next()
            .await?
            .ok_or_else(|| "the server closed before replying to hello".into())
    }

    /// The next record from the server, or `None` once it closed the stream.
    ///
    /// # Errors
    ///
    /// Fails on a malformed record or when the server is silent past [`WAIT`].
    pub async fn next(&mut self) -> TestResult<Option<Value>> {
        let mut buffer = [0_u8; 4096];
        loop {
            if let Some(message) = self.decoder.next_message()? {
                let payload = match message {
                    WireMessage::ControlLine(line) => line,
                    WireMessage::Frame(frame) => frame.payload,
                    other => return Err(format!("unexpected record {other:?}").into()),
                };
                let value: Value = serde_json::from_slice(&payload)?;
                writeln!(std::io::stdout(), "<- {value}")?;
                return Ok(Some(value));
            }
            if self.pending.is_empty() {
                let count = tokio::time::timeout(WAIT, self.stream.read(&mut buffer)).await??;
                if count == 0 {
                    return Ok(None);
                }
                self.pending = buffer.get(..count).ok_or("read past the buffer")?.to_vec();
            }
            let pending = std::mem::take(&mut self.pending);
            if let FeedOutcome::NeedDrain { consumed } = self.decoder.feed(&pending)? {
                self.pending = pending
                    .get(consumed..)
                    .ok_or("consumed past the input")?
                    .to_vec();
            }
        }
    }

    /// The next record, failing the test when the server closed instead.
    ///
    /// # Errors
    ///
    /// Fails as [`Client::next`] does, or when the stream ended.
    pub async fn expect_next(&mut self) -> TestResult<Value> {
        self.next()
            .await?
            .ok_or_else(|| "the connection stays open".into())
    }
}
