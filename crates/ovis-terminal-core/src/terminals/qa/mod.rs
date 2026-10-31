//! Real holder, authority and SQLite fixtures for terminal lifecycle integration.

mod administration;
mod adversarial;
mod authority;
mod behavior;
mod counters;
mod recovery;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use behavior::{failure, happy};
use ovis_terminal_authz::{
    Authority, AuthorityConfig, EndpointPolicy, LocalAccount, OwnerSpec, PeerEvidence,
    Presentation, PrincipalId, RemoteTransport, RuntimeBinding, Session, UnixUid,
};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RegistryIdentities, RuntimeId, RuntimeIdentity,
    ServerBootId,
};
use ovis_terminal_core::terminals::{Call, Host, Launch, LaunchState, Program, Record, Terminals};
use ovis_terminal_holder::protocol::{HolderBuildId, HolderDescriptor, HolderInstanceId};
use ovis_terminal_holder::unix::{Holder, HolderConfig, HolderLimits};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_store::{Store, StoreOptions};
use uuid::Uuid;

/// Outcome of an assertion-bearing lifecycle acceptance case.
pub type TestResult = Result<(), Box<dyn std::error::Error>>;
const WAIT: Duration = Duration::from_secs(15);

struct Fixture {
    root: PathBuf,
    holder: Holder,
    session: Session,
    runtime: Option<Terminals>,
    identity: RuntimeIdentity,
    authority: Arc<Authority<RegistryIdentities>>,
    credential: ovis_terminal_authz::CredentialId,
}

impl Fixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Self::with_options(StoreOptions::default())
    }

    fn with_options(options: StoreOptions) -> Result<Self, Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!("ovis-t16-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&root)?;
        let identity = RuntimeIdentity::new(
            HostIdentity::new(
                MachineId::new(Uuid::from_u128(1))?,
                HostBootId::try_from(1)?,
            ),
            RuntimeId::try_from(1)?,
            ServerBootId::try_from(1)?,
        );
        let (authority, owner) = Authority::<RegistryIdentities>::bootstrap(AuthorityConfig::new(
            RuntimeBinding::new(identity.machine_id(), identity.runtime_id()),
            OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(u64::MAX)),
        ))?;
        let admission = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1)))
            .admit(&PeerEvidence::Remote(RemoteTransport::Ssh))?;
        let session = authority.authenticate(
            &admission,
            Presentation::new(owner.bearer(), TimestampMs::new(1)),
        )?;
        let credential = *owner.id();
        let holder = Holder::new(HolderConfig::new(
            HolderDescriptor::new(
                HolderBuildId::new("task-16".into()),
                HolderInstanceId::new(Uuid::now_v7().to_string()),
            ),
            HolderLimits::DEFAULT,
        ));
        let authority = Arc::new(authority);
        let runtime = Terminals::new(
            Store::open(&root.join("state"), options)?,
            Host::new(identity, Arc::clone(&authority), holder.clone()),
        )?;
        Ok(Self {
            root,
            holder,
            session,
            runtime: Some(runtime),
            identity,
            authority,
            credential,
        })
    }

    fn runtime(&mut self) -> Result<&mut Terminals, Box<dyn std::error::Error>> {
        self.runtime.as_mut().ok_or_else(|| "open runtime".into())
    }

    fn launch(&self, argv: Vec<String>) -> Result<Launch, Box<dyn std::error::Error>> {
        let program = Program::new(
            argv,
            self.root.clone(),
            [
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("OVIS_FIXTURE".into(), "explicit-environment".into()),
            ]
            .into(),
        )?;
        Ok(Launch::new(
            program,
            serde_json::from_value(serde_json::json!({"cols":103,"rows":37}))?,
        )?)
    }

    fn finish(
        &mut self,
        id: ovis_terminal_core::registry::TerminalId,
    ) -> Result<Record, Box<dyn std::error::Error>> {
        let deadline = Instant::now().checked_add(WAIT).ok_or("deadline")?;
        loop {
            let record = self
                .runtime
                .as_ref()
                .ok_or("runtime")?
                .get(&self.session, id)?;
            if record.exit.is_some() || record.spawn == LaunchState::Failed {
                return Ok(record);
            }
            self.runtime()?
                .receive(deadline.saturating_duration_since(Instant::now()))?;
        }
    }

    fn marker(
        &mut self,
        id: ovis_terminal_core::registry::TerminalId,
        marker: &[u8],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let deadline = Instant::now().checked_add(WAIT).ok_or("deadline")?;
        loop {
            if let Ok(tail) = self
                .runtime
                .as_ref()
                .ok_or("runtime")?
                .output(&self.session, id)
                && tail
                    .bytes
                    .windows(marker.len())
                    .any(|bytes| bytes == marker)
            {
                return Ok(tail.bytes);
            }
            self.runtime()?
                .receive(deadline.saturating_duration_since(Instant::now()))?;
        }
    }

    fn reopen(&mut self, boot: u64) -> TestResult {
        self.runtime.take().ok_or("runtime")?.close()?;
        self.identity = RuntimeIdentity::new(
            HostIdentity::new(self.identity.machine_id(), self.identity.host_boot_id()),
            self.identity.runtime_id(),
            ServerBootId::try_from(boot)?,
        );
        self.runtime = Some(Terminals::new(
            Store::open(&self.root.join("state"), StoreOptions::default())?,
            Host::new(
                self.identity,
                Arc::clone(&self.authority),
                self.holder.clone(),
            ),
        )?);
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            let closed = runtime.close();
            assert!(closed.is_ok(), "close control: {closed:?}");
        }
        let stopped = self.holder.shutdown();
        assert!(stopped.is_ok(), "join owned holder workers: {stopped:?}");
        let removed = std::fs::remove_dir_all(&self.root);
        assert!(removed.is_ok(), "remove owned fixture: {removed:?}");
    }
}

fn call(session: &Session) -> Call<'_> {
    Call::new(session, OpId::new(Uuid::now_v7().to_string()))
}
