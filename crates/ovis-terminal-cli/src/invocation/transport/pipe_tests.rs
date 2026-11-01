//! The CLI client against real named pipes: a busy pipe is waited out, a pipe that
//! stays busy is `pipe_busy_timeout`, and two invocations opened at once both get a
//! session from the production pipe listener.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use ovis_terminal_authz::{Authority, AuthorityConfig, OwnerSpec, PrincipalId, RuntimeBinding};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, RuntimeId, RuntimeIdentity, ServerBootId,
};
use ovis_terminal_protocol::domain::windows_service::PipeName;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::rpc::parse_request;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_server::conn::{Call, Connections, RequestHandler};
use ovis_terminal_server::listen::EndpointAddress;
use ovis_terminal_server::listen::pipe::{PipeEndpoint, PipeServerHandle, serve_pipe};
use serde_json::{Value, json};
use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

use super::open;
use crate::invocation::context::Credential;
use crate::invocation::error::CliError;
use crate::invocation::transport::Session;

type TestResult = Result<(), Box<dyn std::error::Error>>;

static NEXT: AtomicU32 = AtomicU32::new(0);

fn unique(prefix: &str) -> String {
    let unique = NEXT.fetch_add(1, Ordering::Relaxed);
    format!(r"\\.\pipe\ovt60-{prefix}-{}-{unique}", std::process::id())
}

#[tokio::test]
async fn a_busy_pipe_is_waited_out_and_opened_once_an_instance_is_free() -> TestResult {
    let name = unique("busy");
    let first = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&name)?;
    let holder = ClientOptions::new().open(&name)?;
    let opening = tokio::spawn({
        let name = name.clone();
        async move { open(&name, Duration::from_secs(20)).await }
    });
    let second = ServerOptions::new().create(&name)?;
    let opened = opening.await?;
    assert!(opened.is_ok(), "{opened:?}");
    drop((holder, first, second));
    Ok(())
}

#[tokio::test]
async fn a_pipe_that_stays_busy_is_pipe_busy_timeout_after_the_bound() -> TestResult {
    let name = unique("taken");
    let only = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&name)?;
    let holder = ClientOptions::new().open(&name)?;
    let refused = open(&name, Duration::from_millis(300)).await;
    let Err(error) = refused else {
        return Err("a pipe whose only instance is taken was opened".into());
    };
    assert_eq!(error.kind(), "pipe_busy_timeout", "{error:?}");
    assert!(
        matches!(error, CliError::PipeBusy { waited_ms: 300, .. }),
        "{error:?}"
    );
    let missing = open(&unique("absent"), Duration::from_millis(300)).await;
    assert!(
        matches!(
            missing,
            Err(CliError::NotRunning {
                state: "stopped",
                ..
            })
        ),
        "{missing:?}"
    );
    drop((holder, only));
    Ok(())
}

/// Counts every call that reaches it and answers with the method name.
#[derive(Debug, Default)]
struct Counting(Arc<AtomicU64>);

impl RequestHandler for Counting {
    fn methods(&self) -> Vec<&'static str> {
        vec!["doc.get"]
    }

    fn call(&self, call: Call) -> impl Future<Output = Result<Value, BoundaryError>> + Send {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::future::ready(Ok(json!({"served": call.request.method()})))
    }
}

/// The production pipe listener on a fresh name, its owner bearer and its call count.
struct Served {
    name: String,
    bearer: String,
    reached: Arc<AtomicU64>,
    server: PipeServerHandle,
}

fn served() -> Result<Served, Box<dyn std::error::Error>> {
    let machine = "machine:0191f1a2-0000-7000-8000-000000000060".parse()?;
    let runtime = RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1)?),
        RuntimeId::try_from(6)?,
        ServerBootId::try_from(1)?,
    );
    let owner = OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(u64::MAX));
    let binding = RuntimeBinding::new(machine, runtime.runtime_id());
    let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(binding, owner))?;
    let reached = Arc::new(AtomicU64::new(0));
    let handler = Arc::new(Counting(Arc::clone(&reached)));
    let connections = Connections::new(Arc::new(authority), runtime, handler)?;
    let name = unique("served");
    let endpoint = PipeEndpoint::bind(&PipeName::parse(&name)?)?;
    Ok(Served {
        name,
        bearer: issued.bearer().to_owned(),
        reached,
        server: serve_pipe(endpoint, connections),
    })
}

#[tokio::test]
async fn two_invocations_opened_at_once_both_get_a_session_from_the_production_listener()
-> TestResult {
    let Served {
        name,
        bearer,
        reached,
        server,
    } = served()?;
    let address = EndpointAddress::Pipe(name);
    let credential = Credential::new(bearer);
    let (first, second) = tokio::join!(
        Session::open(&address, Some(&credential)),
        Session::open(&address, Some(&credential))
    );
    let mut first = first.map_err(|failure| format!("first invocation: {failure:?}"))?;
    let mut second = second.map_err(|failure| format!("second invocation: {failure:?}"))?;
    let get = parse_request(r#"{"jsonrpc":"2.0","id":"g","method":"doc.get","params":{}}"#)?;
    let (a, b) = tokio::join!(first.call(&get), second.call(&get));
    let a = a.map_err(|failure| format!("first call: {failure:?}"))?;
    let b = b.map_err(|failure| format!("second call: {failure:?}"))?;
    assert_eq!(a, json!({"served": "doc.get"}));
    assert_eq!(b, json!({"served": "doc.get"}));
    assert_eq!(reached.load(Ordering::SeqCst), 2);
    drop((first, second));
    let report = server.shutdown().await?;
    assert_eq!((report.accepted(), report.failures().len()), (2, 0));
    Ok(())
}
