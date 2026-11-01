//! One production pipe server behind the boundary policy, with an owner and an
//! observer credential, and the line-oriented client helpers.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::{
    Authority, AuthorityConfig, EndpointPolicy, Grant, Issuance, IssueRequest, LocalAccount,
    OwnerSpec, PeerEvidence, Permission, Presentation, PrincipalId, RemoteTransport, ResourceScope,
    RuntimeBinding, ScopeResource, UnixUid,
};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, RegistryIdentities, RuntimeId, RuntimeIdentity, ServerBootId,
};
use ovis_terminal_core::telemetry::redaction::RetentionPolicy;
use ovis_terminal_protocol::domain::windows_service::PipeName;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_server::conn::{Call, Connections, RequestHandler};
use ovis_terminal_server::dispatch::lanes::boundary_policy::{
    AuditTrail, BoundaryPolicy, MUTATION_ROLES,
};
use ovis_terminal_server::listen::pipe::{PipeEndpoint, PipeServerHandle, serve_pipe};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;
use windows_sys::Win32::System::Pipes::WaitNamedPipeW;

static NEXT: AtomicU32 = AtomicU32::new(0);
const BOUND: Duration = Duration::from_secs(20);

/// Counts every call that reaches it and answers with the method name.
#[derive(Debug, Default)]
pub struct Counting(Arc<AtomicU64>);

impl RequestHandler for Counting {
    fn methods(&self) -> Vec<&'static str> {
        let mut methods: Vec<&'static str> = MUTATION_ROLES.iter().map(|(name, _)| *name).collect();
        methods.push("doc.get");
        methods
    }

    fn call(&self, call: Call) -> impl Future<Output = Result<Value, BoundaryError>> + Send {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::future::ready(Ok(json!({"served": call.request.method()})))
    }
}

/// A serving pipe and the two credentials it accepts.
#[derive(Debug)]
#[non_exhaustive]
pub struct Served {
    /// The pipe's name.
    pub name: PipeName,
    /// The serving listener.
    pub server: PipeServerHandle,
    /// Calls that reached the handler behind the policy.
    pub reached: Arc<AtomicU64>,
    /// The bootstrap owner's bearer.
    pub owner: String,
    /// An observe-only bearer.
    pub observer: String,
}

fn runtime() -> RuntimeIdentity {
    let machine = "machine:0191f1a2-0000-7000-8000-000000000060"
        .parse()
        .unwrap();
    RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1).unwrap()),
        RuntimeId::try_from(6).unwrap(),
        ServerBootId::try_from(1).unwrap(),
    )
}

fn now() -> TimestampMs {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    TimestampMs::new(u64::try_from(ms).unwrap())
}

/// Issues an observe-only credential through the owner's session.
fn observer(
    authority: &Authority<RegistryIdentities>,
    (owner, runtime): (&str, RuntimeIdentity),
) -> String {
    let admission = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1000)))
        .admit(&PeerEvidence::Remote(RemoteTransport::Ssh))
        .unwrap();
    let session = authority
        .authenticate(&admission, Presentation::new(owner, now()))
        .unwrap();
    let scope = ResourceScope::new(runtime.machine_id(), ScopeResource::Machine);
    let request = IssueRequest::new(
        PrincipalId::parse("observer").unwrap(),
        vec![Grant::new(Permission::Observe, scope)],
        TimestampMs::new(u64::MAX),
    );
    let issued = authority
        .issue(&session, Issuance::new(request, now()))
        .unwrap();
    issued.bearer().to_owned()
}

impl Served {
    /// Binds a fresh owner-only pipe and serves it.
    ///
    /// # Panics
    ///
    /// The authority, the credentials or the pipe cannot be set up.
    pub fn start() -> Self {
        let runtime = runtime();
        let owner_spec = OwnerSpec::new(
            PrincipalId::parse("owner").unwrap(),
            TimestampMs::new(u64::MAX),
        );
        let binding = RuntimeBinding::new(runtime.machine_id(), runtime.runtime_id());
        let (authority, issued) =
            Authority::bootstrap(AuthorityConfig::new(binding, owner_spec)).unwrap();
        let owner = issued.bearer().to_owned();
        let observer = observer(&authority, (&owner, runtime));
        let authority = Arc::new(authority);
        let retention = RetentionPolicy::new(NonZeroUsize::MIN.saturating_add(255), 60_000);
        let trail = Arc::new(AuditTrail::new(runtime, retention));
        let reached = Arc::new(AtomicU64::new(0));
        let policy = BoundaryPolicy::new(
            (Arc::clone(&authority), trail),
            Counting(Arc::clone(&reached)),
        );
        let connections = Connections::new(authority, runtime, Arc::new(policy)).unwrap();
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!(r"\\.\pipe\ovt60-{}-{unique}", std::process::id());
        let endpoint = PipeEndpoint::bind(&PipeName::parse(&name).unwrap()).unwrap();
        Self {
            name: endpoint.name().clone(),
            server: serve_pipe(endpoint, connections),
            reached,
            owner,
            observer,
        }
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

/// A test failure carrying its typed cause.
pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// One connected line-oriented client.
#[derive(Debug)]
pub struct Client(BufReader<NamedPipeClient>);

impl Client {
    /// Opens `name` the way the Windows pipe contract requires of a client: an open
    /// refused with `ERROR_PIPE_BUSY` waits with `WaitNamedPipeW` for the server's next
    /// instance, bounded by what is left of [`BOUND`], and opens again.
    ///
    /// # Errors
    ///
    /// Every instance stayed busy for the whole bound, or the open failed otherwise.
    pub async fn open(name: &PipeName) -> Result<Self, OpenError> {
        let started = std::time::Instant::now();
        loop {
            let error = match ClientOptions::new().open(name.as_str()) {
                Ok(pipe) => return Ok(Self(BufReader::new(pipe))),
                Err(error) => error,
            };
            let busy = error
                .raw_os_error()
                .and_then(|code| u32::try_from(code).ok())
                == Some(ERROR_PIPE_BUSY);
            let left = BOUND.saturating_sub(started.elapsed());
            if !busy {
                return Err(OpenError::Io {
                    name: name.as_str().to_owned(),
                    source: error,
                });
            }
            if left.is_zero() {
                return Err(OpenError::Busy {
                    name: name.as_str().to_owned(),
                    source: error,
                });
            }
            let wide: Vec<u16> = name.as_str().encode_utf16().chain(Some(0)).collect();
            let timeout = u32::try_from(left.as_millis()).unwrap_or(u32::MAX);
            tokio::task::spawn_blocking(move || {
                // SAFETY: `wide` is NUL-terminated and lives for the whole call.
                unsafe { WaitNamedPipeW(wide.as_ptr(), timeout) }
            })
            .await
            .map_err(|join| OpenError::Io {
                name: name.as_str().to_owned(),
                source: std::io::Error::other(join),
            })?;
        }
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

/// Why a client could not open the pipe.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OpenError {
    /// Every instance stayed busy for the whole bound.
    #[error("every instance of {name} stayed busy")]
    Busy {
        /// The pipe name.
        name: String,
        /// The last refusal, `ERROR_PIPE_BUSY`.
        #[source]
        source: std::io::Error,
    },
    /// The open failed for another reason.
    #[error("could not open {name}")]
    Io {
        /// The pipe name.
        name: String,
        /// The operating system error.
        #[source]
        source: std::io::Error,
    },
}

impl OpenError {
    /// Stable machine-readable kind.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Busy { .. } => "pipe_busy",
            Self::Io { .. } => "pipe_open_failed",
        }
    }

    /// What to do about it.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Busy { .. } => "the server never offered a free instance; check its accept loop",
            Self::Io { .. } => "check the pipe exists and this account may open it",
        }
    }
}
