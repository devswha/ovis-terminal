//! The production pipe listener behind the production boundary policy, with an owner
//! and an observe-only credential, for tests that open it through `Session::open`.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

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
use ovis_terminal_server::listen::EndpointAddress;
use ovis_terminal_server::listen::pipe::{PipeEndpoint, PipeServerHandle, serve_pipe};
use serde_json::{Value, json};

use super::{TestResult, unique};
use crate::invocation::context::Credential;

/// Counts every call that reaches it and answers with the method name.
#[derive(Debug, Default)]
pub(super) struct Counting(pub(super) Arc<AtomicU64>);

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

/// A serving pipe, both credentials and the count of calls past the policy.
pub(super) struct Served {
    pub(super) address: EndpointAddress,
    pub(super) owner: Credential,
    pub(super) observer: Credential,
    pub(super) reached: Arc<AtomicU64>,
    pub(super) server: PipeServerHandle,
}

fn runtime() -> TestResult<RuntimeIdentity> {
    let machine = "machine:0191f1a2-0000-7000-8000-000000000060".parse()?;
    Ok(RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1)?),
        RuntimeId::try_from(6)?,
        ServerBootId::try_from(1)?,
    ))
}

fn now() -> TestResult<TimestampMs> {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    Ok(TimestampMs::new(u64::try_from(ms)?))
}

/// An observe-only bearer issued through the owner's session.
fn observer(
    authority: &Authority<RegistryIdentities>,
    (owner, runtime): (&str, RuntimeIdentity),
) -> TestResult<String> {
    let admission = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1000)))
        .admit(&PeerEvidence::Remote(RemoteTransport::Ssh))?;
    let session = authority.authenticate(&admission, Presentation::new(owner, now()?))?;
    let scope = ResourceScope::new(runtime.machine_id(), ScopeResource::Machine);
    let request = IssueRequest::new(
        PrincipalId::parse("observer")?,
        vec![Grant::new(Permission::Observe, scope)],
        TimestampMs::new(u64::MAX),
    );
    Ok(authority
        .issue(&session, Issuance::new(request, now()?))?
        .bearer()
        .to_owned())
}

/// Binds a fresh owner-only pipe and serves it behind the boundary policy.
pub(super) fn served() -> TestResult<Served> {
    let runtime = runtime()?;
    let spec = OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(u64::MAX));
    let binding = RuntimeBinding::new(runtime.machine_id(), runtime.runtime_id());
    let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(binding, spec))?;
    let owner = issued.bearer().to_owned();
    let observer = observer(&authority, (&owner, runtime))?;
    let authority = Arc::new(authority);
    let retention = RetentionPolicy::new(NonZeroUsize::MIN.saturating_add(255), 60_000);
    let trail = Arc::new(AuditTrail::new(runtime, retention));
    let reached = Arc::new(AtomicU64::new(0));
    let handler = Counting(Arc::clone(&reached));
    let policy = BoundaryPolicy::new((Arc::clone(&authority), trail), handler);
    let connections = Connections::new(authority, runtime, Arc::new(policy))?;
    let name = unique("served");
    let endpoint = PipeEndpoint::bind(&PipeName::parse(&name)?)?;
    Ok(Served {
        address: EndpointAddress::Pipe(name),
        owner: Credential::new(owner),
        observer: Credential::new(observer),
        reached,
        server: serve_pipe(endpoint, connections),
    })
}
