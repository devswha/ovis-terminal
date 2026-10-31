//! A real holder, SQLite store and authority under one restartable control.

mod barriers;
mod fates;
mod happy;
mod loss;
mod stop;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ovis_terminal_authz::{
    Authority, AuthorityConfig, EndpointPolicy, LocalAccount, OwnerSpec, PeerEvidence,
    Presentation, PrincipalId, RemoteTransport, RuntimeBinding, Session, UnixUid,
};
use ovis_terminal_holder::process::{ProcessId, read_start_token};
use ovis_terminal_holder::protocol::{HolderBuildId, HolderDescriptor, HolderInstanceId};
use ovis_terminal_holder::unix::{Holder, HolderConfig, HolderLimits};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use uuid::Uuid;

use super::{Control, ControlPlan};
use crate::registry::{
    HostBootId, HostIdentity, MachineId, ProcessIdentity, RegistryIdentities, RuntimeId,
    RuntimeIdentity, ServerBootId, TerminalId,
};
use crate::terminals::{Call, Launch, LaunchState, Program, Record};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
const WAIT: Duration = Duration::from_secs(15);

struct Fixture {
    root: PathBuf,
    holder: Holder,
    authority: Arc<Authority<RegistryIdentities>>,
    bearer: String,
    identity: RuntimeIdentity,
}

impl Fixture {
    fn new() -> TestResult<Self> {
        let root = std::env::temp_dir().join(format!("ovis-t48-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&root)?;
        let identity = RuntimeIdentity::new(
            HostIdentity::new(
                MachineId::new(Uuid::from_u128(48))?,
                HostBootId::try_from(1)?,
            ),
            RuntimeId::try_from(7)?,
            ServerBootId::try_from(1)?,
        );
        let (authority, owner) = Authority::bootstrap(AuthorityConfig::new(
            RuntimeBinding::new(identity.machine_id(), identity.runtime_id()),
            OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(u64::MAX)),
        ))?;
        let holder = Holder::new(HolderConfig::new(
            HolderDescriptor::new(
                HolderBuildId::new("task-48".into()),
                HolderInstanceId::new(Uuid::now_v7().to_string()),
            ),
            HolderLimits::DEFAULT,
        ));
        Ok(Self {
            root,
            holder,
            authority: Arc::new(authority),
            bearer: owner.bearer().to_owned(),
            identity,
        })
    }

    fn session(&self) -> TestResult<Session> {
        let admission = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1)))
            .admit(&PeerEvidence::Remote(RemoteTransport::Ssh))?;
        Ok(self.authority.authenticate(
            &admission,
            Presentation::new(&self.bearer, TimestampMs::new(1)),
        )?)
    }

    fn plan(&self) -> TestResult<ControlPlan> {
        Ok(ControlPlan::new(
            self.root.clone(),
            (self.holder.clone(), Arc::clone(&self.authority)),
            self.session()?,
        ))
    }

    /// Opens control of the fixture identity; a pending intent is finished first.
    fn open(&self) -> TestResult<(Control, super::Boot)> {
        Ok(Control::open(self.plan()?, self.identity)?)
    }

    fn launch(&self, script: &str) -> TestResult<Launch> {
        let program = Program::new(
            vec!["/bin/sh".into(), "-c".into(), script.into()],
            self.root.clone(),
            [("PATH".into(), "/usr/bin:/bin".into())].into(),
        )?;
        Ok(Launch::new(
            program,
            serde_json::from_value(serde_json::json!({"cols": 80, "rows": 24}))?,
        )?)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let stopped = self.holder.shutdown();
        assert!(stopped.is_ok(), "join owned holder workers: {stopped:?}");
        let removed = std::fs::remove_dir_all(&self.root);
        assert!(removed.is_ok(), "remove owned fixture: {removed:?}");
    }
}

/// The receipt of a restart the open finished from its intent; an unplanned boot fails.
fn planned(boot: super::Boot) -> TestResult<super::RestartReceipt> {
    match boot {
        super::Boot::Planned(receipt) => Ok(*receipt),
        other => Err(format!("expected the planned path, got {other:?}").into()),
    }
}

fn op() -> OpId {
    OpId::new(Uuid::now_v7().to_string())
}

/// Starts `script` and returns its record once the holder proved the process.
fn spawn(control: &Control, launch: Launch) -> TestResult<Record> {
    let record = control.with_terminals(|terminals, session| {
        let admitted = terminals.run(Call::new(session, op()), launch)?;
        terminals.await_spawn(admitted.id(), WAIT)
    })?;
    assert_eq!(record.spawn, LaunchState::Spawned);
    Ok(record)
}

/// Receives holder evidence until `id`'s retained output holds `marker`.
fn marker(control: &Control, (id, marker): (TerminalId, &[u8])) -> TestResult {
    let deadline = Instant::now().checked_add(WAIT).ok_or("deadline")?;
    loop {
        let found = control.with_terminals(|terminals, session| {
            let tail = terminals.output(session, id)?;
            if !tail
                .bytes
                .windows(marker.len())
                .any(|bytes| bytes == marker)
            {
                terminals.receive(deadline.saturating_duration_since(Instant::now()))?;
                return Ok(false);
            }
            Ok(true)
        })?;
        if found {
            return Ok(());
        }
    }
}

/// Whether the OS still runs `process` with the start token the holder reported.
fn running(process: &ProcessIdentity) -> TestResult<bool> {
    let pid = u32::try_from(process.pid().get())?;
    assert!(pid > 1, "only a spawned child is inspected");
    Ok(read_start_token(ProcessId::new(pid))
        .is_ok_and(|token| token.as_str() == process.start_token().as_str()))
}
