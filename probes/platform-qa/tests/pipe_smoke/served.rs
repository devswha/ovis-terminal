//! One production pipe server with its owner credential, and a raw line client that
//! opens the pipe once (a single client never meets a busy instance).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use ovis_terminal_authz::{Authority, AuthorityConfig, OwnerSpec, PrincipalId, RuntimeBinding};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, RuntimeId, RuntimeIdentity, ServerBootId,
};
use ovis_terminal_protocol::domain::windows_service::PipeName;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_server::conn::{Call, Connections, RequestHandler};
use ovis_terminal_server::listen::pipe::{PipeEndpoint, PipeServerHandle, serve_pipe};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

static NEXT: AtomicU32 = AtomicU32::new(0);
const BOUND: Duration = Duration::from_secs(20);

/// A test failure carrying its typed cause.
pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Counts every call that reaches it and answers with the method name.
#[derive(Debug, Default)]
pub struct Counting(Arc<AtomicU64>);

impl RequestHandler for Counting {
    fn methods(&self) -> Vec<&'static str> {
        vec!["doc.get", "terminal.run"]
    }

    fn call(&self, call: Call) -> impl Future<Output = Result<Value, BoundaryError>> + Send {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::future::ready(Ok(json!({"served": call.request.method()})))
    }
}

/// A serving pipe and the owner credential it accepts.
#[derive(Debug)]
#[non_exhaustive]
pub struct Served {
    /// The pipe's name.
    pub name: PipeName,
    /// The serving listener.
    pub server: PipeServerHandle,
    /// Calls that reached the handler.
    pub reached: Arc<AtomicU64>,
    /// The bootstrap owner's bearer.
    pub owner: String,
}

impl Served {
    /// Binds a fresh owner-only pipe and serves it.
    ///
    /// # Errors
    ///
    /// The authority, the identity or the pipe cannot be set up.
    pub fn start() -> TestResult<Self> {
        let machine = "machine:0191f1a2-0000-7000-8000-000000000060".parse()?;
        let runtime = RuntimeIdentity::new(
            HostIdentity::new(machine, HostBootId::try_from(1)?),
            RuntimeId::try_from(6)?,
            ServerBootId::try_from(1)?,
        );
        let spec = OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(u64::MAX));
        let binding = RuntimeBinding::new(machine, runtime.runtime_id());
        let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(binding, spec))?;
        let reached = Arc::new(AtomicU64::new(0));
        let handler = Arc::new(Counting(Arc::clone(&reached)));
        let connections = Connections::new(Arc::new(authority), runtime, handler)?;
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!(r"\\.\pipe\ovt60-{}-{unique}", std::process::id());
        let endpoint = PipeEndpoint::bind(&PipeName::parse(&name)?)?;
        Ok(Self {
            name: endpoint.name().clone(),
            server: serve_pipe(endpoint, connections),
            reached,
            owner: issued.bearer().to_owned(),
        })
    }
}

/// One JSON-RPC record as a newline-terminated line.
pub fn line(record: &Value) -> String {
    format!("{record}\n")
}

/// A hello line with `generation` exactly as given and `credential`.
pub fn hello(generation: &Value, credential: &str) -> String {
    line(
        &json!({"jsonrpc": "2.0", "id": "h", "method": "hello", "params": {
        "generation": generation, "client": {"name": "platform-qa", "version": "1"},
        "credential": credential}}),
    )
}

/// One connected line-oriented client.
#[derive(Debug)]
pub struct Client(BufReader<NamedPipeClient>);

impl Client {
    /// Opens `name` once.
    ///
    /// # Errors
    ///
    /// The pipe cannot be opened.
    pub fn open(name: &PipeName) -> TestResult<Self> {
        Ok(Self(BufReader::new(
            ClientOptions::new().open(name.as_str())?,
        )))
    }

    /// Writes `lines` and reads `replies` reply lines, each within [`BOUND`].
    ///
    /// # Errors
    ///
    /// The pipe fails, a reply is late or a reply line is not JSON.
    pub async fn exchange(&mut self, lines: &str, replies: usize) -> TestResult<Vec<Value>> {
        self.0.get_mut().write_all(lines.as_bytes()).await?;
        let mut answered = Vec::with_capacity(replies);
        for _ in 0..replies {
            let mut text = String::new();
            tokio::time::timeout(BOUND, self.0.read_line(&mut text)).await??;
            answered.push(serde_json::from_str(&text)?);
        }
        Ok(answered)
    }
}
