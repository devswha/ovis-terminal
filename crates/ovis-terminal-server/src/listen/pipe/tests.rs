//! The pipe listener on a real named pipe: admission, hello, persistence and refusals.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use ovis_terminal_authz::{Authority, AuthorityConfig, OwnerSpec, PrincipalId, RuntimeBinding};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, RuntimeId, RuntimeIdentity, ServerBootId,
};
use ovis_terminal_holder::service::windows::pipe;
use ovis_terminal_protocol::domain::windows_service::PipeName;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

use super::{PipeEndpoint, PipeServerHandle, serve_pipe};
use crate::conn::{Call, Connections, RequestHandler};

static NEXT: AtomicU32 = AtomicU32::new(0);
const BOUND: Duration = Duration::from_secs(20);

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

fn unique_pipe() -> PipeName {
    let unique = NEXT.fetch_add(1, Ordering::Relaxed);
    PipeName::parse(&format!(r"\\.\pipe\ovt54-{}-{unique}", std::process::id())).unwrap()
}

struct Served {
    name: PipeName,
    server: PipeServerHandle,
    handler: Arc<Counting>,
    bearer: String,
}

fn serve(endpoint: PipeEndpoint) -> Served {
    let machine = "machine:0191f1a2-0000-7000-8000-000000000054"
        .parse()
        .unwrap();
    let runtime = RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1).unwrap()),
        RuntimeId::try_from(5).unwrap(),
        ServerBootId::try_from(1).unwrap(),
    );
    let owner = OwnerSpec::new(
        PrincipalId::parse("owner").unwrap(),
        TimestampMs::new(u64::MAX),
    );
    let binding = RuntimeBinding::new(machine, runtime.runtime_id());
    let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(binding, owner)).unwrap();
    let handler = Arc::new(Counting::default());
    let connections = Connections::new(Arc::new(authority), runtime, Arc::clone(&handler)).unwrap();
    let name = endpoint.name().clone();
    Served {
        name,
        server: serve_pipe(endpoint, connections),
        handler,
        bearer: issued.bearer().to_owned(),
    }
}

fn hello(credential: &str) -> String {
    let request = json!({"jsonrpc": "2.0", "id": "h", "method": "hello", "params": {
        "generation": "1", "client": {"name": "t", "version": "1"}, "credential": credential}});
    format!("{request}\n")
}

async fn exchange(client: NamedPipeClient, lines: &str, replies: usize) -> Vec<Value> {
    let mut reader = BufReader::new(client);
    reader.get_mut().write_all(lines.as_bytes()).await.unwrap();
    let mut answered = Vec::new();
    for _ in 0..replies {
        let mut line = String::new();
        tokio::time::timeout(BOUND, reader.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        answered.push(serde_json::from_str(&line).unwrap());
    }
    answered
}

#[tokio::test]
async fn hello_and_pipelined_requests_share_one_owner_only_pipe_connection() {
    let served = serve(PipeEndpoint::bind(&unique_pipe()).unwrap());
    let client = ClientOptions::new().open(served.name.as_str()).unwrap();
    let security = pipe::instance_security(&client).unwrap();
    let owner = pipe::current_user_sid().unwrap();
    assert_eq!(security.owner, owner);
    let dacl = security.dacl.expect("an explicit DACL, never a null one");
    assert_eq!(dacl.len(), 1, "{dacl:?}");
    assert!(
        dacl.iter().all(|ace| ace.allow && ace.trustee == owner),
        "{dacl:?}"
    );
    let get = |id: &str| {
        format!(
            "{}\n",
            json!({"jsonrpc": "2.0", "id": id, "method": "doc.get", "params": {}})
        )
    };
    let lines = format!(
        "{}{}{}{}",
        hello(&served.bearer),
        get("a"),
        get("b"),
        get("c")
    );
    let replies = exchange(client, &lines, 4).await;
    let (first, rest) = replies.split_first().unwrap();
    assert!(first.pointer("/result/server_boot_id").is_some(), "{first}");
    let mut ids: Vec<_> = rest
        .iter()
        .filter_map(|r| r.get("id")?.as_str().map(str::to_owned))
        .collect();
    ids.sort();
    assert_eq!(ids, ["a", "b", "c"]);
    assert_eq!(served.handler.0.load(Ordering::SeqCst), 3);
    let report = served.server.shutdown().await.unwrap();
    assert_eq!(
        (report.accepted(), report.failures().len()),
        (1, 0),
        "{:?}",
        report.failures()
    );
}

#[tokio::test]
async fn a_credential_of_another_runtime_is_refused_at_hello_and_nothing_runs() {
    let served = serve(PipeEndpoint::bind(&unique_pipe()).unwrap());
    let client = ClientOptions::new().open(served.name.as_str()).unwrap();
    let get = json!({"jsonrpc": "2.0", "id": "g", "method": "doc.get", "params": {}});
    let replies = exchange(
        client,
        &format!("{}{get}\n", hello("other-runtime-token")),
        1,
    )
    .await;
    let reply = replies.first().unwrap();
    let kind = reply.pointer("/error/data/kind").and_then(Value::as_str);
    assert!(
        matches!(kind, Some("invalid_credential" | "unauthenticated")),
        "{reply}"
    );
    assert!(reply.get("result").is_none());
    assert_eq!(served.handler.0.load(Ordering::SeqCst), 0);
    served.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_second_listener_never_joins_a_claimed_name() {
    let name = unique_pipe();
    let first = PipeEndpoint::bind(&name).unwrap();
    let second = PipeEndpoint::bind(&name)
        .map(drop)
        .map_err(|error| error.kind());
    assert_eq!(second, Err("already_running"));
    drop(first);
}

#[tokio::test]
async fn cancelled_pending_accepts_leave_the_endpoint_serving() {
    let mut endpoint = PipeEndpoint::bind(&unique_pipe()).unwrap();
    for _ in 0..50 {
        let cancelled = tokio::time::timeout(Duration::ZERO, endpoint.accept()).await;
        assert!(cancelled.is_err(), "an accept completed with no client");
    }
    let served = serve(endpoint);
    let client = ClientOptions::new().open(served.name.as_str()).unwrap();
    let replies = exchange(client, &hello(&served.bearer), 1).await;
    let reply = replies.first().unwrap();
    assert!(reply.pointer("/result/server_boot_id").is_some(), "{reply}");
    served.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_anonymous_token_cannot_open_the_pipe() {
    let served = serve(PipeEndpoint::bind(&unique_pipe()).unwrap());
    let name = served.name.clone();
    let opened = std::thread::spawn(move || anonymous::open(&name))
        .join()
        .unwrap();
    assert_eq!(
        opened.map(drop).map_err(|e| e.raw_os_error()),
        Err(Some(5)),
        "access denied"
    );
    served.server.shutdown().await.unwrap();
}

mod anonymous {
    use std::os::windows::io::{FromRawHandle as _, OwnedHandle};

    use ovis_terminal_protocol::domain::windows_service::PipeName;
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Security::{ImpersonateAnonymousToken, RevertToSelf};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, OPEN_EXISTING};
    use windows_sys::Win32::System::Threading::GetCurrentThread;

    /// Opens `name` while this thread impersonates the anonymous account.
    pub(super) fn open(name: &PipeName) -> std::io::Result<OwnedHandle> {
        let wide: Vec<u16> = name.as_str().encode_utf16().chain([0]).collect();
        // SAFETY: the pseudo-handle of the current thread needs no closing.
        let thread = unsafe { GetCurrentThread() };
        // SAFETY: `thread` is this thread; impersonation is reverted below.
        assert_ne!(unsafe { ImpersonateAnonymousToken(thread) }, 0);
        // SAFETY: `wide` is NUL-terminated; no security attributes or template are passed.
        let raw = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        let error = std::io::Error::last_os_error();
        // SAFETY: ends the impersonation started above.
        assert_ne!(unsafe { RevertToSelf() }, 0);
        if raw == INVALID_HANDLE_VALUE {
            return Err(error);
        }
        // SAFETY: CreateFileW returned a new handle nothing else owns.
        Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
    }
}
