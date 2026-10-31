//! Real PTY programs for `terminal.process_info` (plan todo 55).
//!
//! The fixture launches terminal `proc` through the core lifecycle service and an
//! in-process holder: a job-control shell whose foreground job is an interpreter and
//! whose background job lives only as long as the shell. `fixture.replace_process` stands in for
//! `terminal.kill` plus `terminal.recover` until assembly serves them (todo 31): it
//! stops the exact incarnation, relaunches the program and hands the new incarnation
//! its own agent report token, retiring the old one.

pub mod directory;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(crate) use directory::ProcessDirectoryFixture;
use ovis_terminal_agent::status::{ProcessOwner, StatusService};
use ovis_terminal_authz::{
    Authority, EndpointPolicy, LocalAccount, PeerEvidence, Presentation, RemoteTransport, Session,
    UnixUid,
};
use ovis_terminal_core::registry::{RegistryIdentities, RuntimeIdentity, TerminalId};
use ovis_terminal_core::terminals::{Call, Host, KillRequest, Launch, Program, Terminals};
use ovis_terminal_holder::protocol::{HolderBuildId, HolderDescriptor, HolderInstanceId};
use ovis_terminal_holder::unix::{Holder, HolderConfig, HolderLimits};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::ProcessToken;
use ovis_terminal_protocol::schema::scalar::{Incarnation, TimestampMs};
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};

use super::serve::at;
use crate::error::RunnerError;

/// Terminal id the lifecycle service assigns to the first launch in a fresh store.
pub(crate) const PROC_TERMINAL: u64 = 1;
const READY: &[u8] = b"FG_READY";
const BOUND: Duration = Duration::from_secs(15);

/// `set -m` puts each job in its own process group and hands the terminal to the
/// foreground job. Only the shell holds the fifo's write end, so the background `cat`
/// reads end of file and exits once the shell is gone.
const SCRIPT: &str = "rm -f guard; mkfifo guard; exec 3<>guard; set -m; /bin/cat <guard 3>&- & /bin/sh -c 'printf FG_READY; read gate' 3>&-";

pub(crate) fn argv() -> Vec<String> {
    ["/bin/sh", "-c", SCRIPT].map(str::to_owned).to_vec()
}

/// The token handed to program incarnation `incarnation` of `proc`.
pub(crate) fn token(incarnation: u64) -> ProcessToken {
    ProcessToken::new(format!("fixture-process-token-proc-{incarnation}"))
}

/// The lifecycle service, its owner session and what the fixture saw at each spawn.
#[derive(Debug)]
pub(crate) struct Programs {
    terminals: Mutex<Option<Terminals>>,
    holder: Holder,
    session: Session,
    cwd: String,
    started: Mutex<BTreeMap<u64, TimestampMs>>,
    service: Arc<StatusService>,
    revocation_failed: AtomicBool,
}

fn now() -> Result<TimestampMs, RunnerError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(at("read the clock"))?
        .as_millis();
    Ok(TimestampMs::new(
        u64::try_from(millis).map_err(at("convert the clock"))?,
    ))
}

fn call(session: &Session) -> Call<'_> {
    Call::new(session, OpId::new(uuid::Uuid::now_v7().to_string()))
}

impl Programs {
    /// Launches `proc` and waits, bounded, until its foreground interpreter runs.
    pub(crate) fn start(
        root: &Path,
        (runtime, authority, bearer): (RuntimeIdentity, Arc<Authority<RegistryIdentities>>, String),
        service: Arc<StatusService>,
    ) -> Result<Arc<Self>, RunnerError> {
        let admission = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1)))
            .admit(&PeerEvidence::Remote(RemoteTransport::Ssh))
            .map_err(at("admit the fixture session"))?;
        let session = authority
            .authenticate(&admission, Presentation::new(&bearer, now()?))
            .map_err(at("authenticate the fixture session"))?;
        let holder = Holder::new(HolderConfig::new(
            HolderDescriptor::new(
                HolderBuildId::new("conformance-process-info".to_owned()),
                HolderInstanceId::new(uuid::Uuid::now_v7().to_string()),
            ),
            HolderLimits::DEFAULT,
        ));
        let store = Store::open(&root.join("terminals.db"), StoreOptions::default())
            .map_err(at("open the terminal store"))?;
        let terminals = Terminals::new(store, Host::new(runtime, authority, holder.clone()))
            .map_err(at("open the lifecycle service"))?;
        let cwd = root
            .canonicalize()
            .map_err(at("resolve the fixture root"))?;
        let programs = Arc::new(Self {
            terminals: Mutex::new(Some(terminals)),
            holder,
            session,
            cwd: cwd
                .to_str()
                .ok_or_else(|| at("encode the fixture root")("not UTF-8"))?
                .to_owned(),
            started: Mutex::new(BTreeMap::new()),
            service,
            revocation_failed: AtomicBool::new(false),
        });
        programs.launch()?;
        Ok(programs)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Option<Terminals>>, RunnerError> {
        self.terminals
            .lock()
            .map_err(|_| at("lock the lifecycle service")("poisoned"))
    }

    fn launch(&self) -> Result<(), RunnerError> {
        let env = BTreeMap::from([("PATH".to_owned(), "/usr/bin:/bin".to_owned())]);
        let program =
            Program::new(argv(), self.cwd.clone().into(), env).map_err(at("build the program"))?;
        let geometry = serde_json::from_value(json!({"cols": 80, "rows": 24}))
            .map_err(at("build the geometry"))?;
        let mut launch = Launch::new(program, geometry).map_err(at("build the launch"))?;
        launch.keep = true;
        let mut guard = self.lock()?;
        let launched = guard
            .as_mut()
            .ok_or_else(|| at("launch")("closed"))
            .and_then(|terminals| self.launch_in(terminals, launch));
        drop(guard);
        launched
    }

    fn launch_in(&self, terminals: &mut Terminals, launch: Launch) -> Result<(), RunnerError> {
        let admitted = terminals
            .run(call(&self.session), launch)
            .map_err(at("launch proc"))?;
        if admitted.id()
            != TerminalId::try_from(PROC_TERMINAL).map_err(at("build the terminal id"))?
        {
            return Err(at("launch proc")(
                "the first terminal id is not the scenario's terminal id",
            ));
        }
        self.settle(terminals, admitted.id())
    }

    /// Waits for spawn evidence and the ready marker in this incarnation's own output.
    fn settle(&self, terminals: &mut Terminals, id: TerminalId) -> Result<(), RunnerError> {
        let spawned = terminals
            .await_spawn(id, BOUND)
            .map_err(at("await the spawn"))?;
        let deadline = Instant::now()
            .checked_add(BOUND)
            .ok_or_else(|| at("bound")("overflow"))?;
        loop {
            let tail = terminals
                .output(&self.session, id)
                .map_err(at("read the output"))?;
            if tail.bytes.windows(READY.len()).any(|bytes| bytes == READY) {
                break;
            }
            terminals
                .receive(deadline.saturating_duration_since(Instant::now()))
                .map_err(at("await the ready marker"))?;
        }
        let incarnation = spawned
            .lifecycle
            .last_process()
            .ok_or_else(|| at("spawn")("no process"))?
            .generation()
            .get();
        self.started
            .lock()
            .map_err(|_| at("record the spawn")("poisoned"))?
            .insert(incarnation, now()?);
        self.service
            .tokens()
            .bind(
                &token(incarnation),
                ProcessOwner::new(id, Incarnation::new(incarnation)),
            )
            .map_err(at("hand the agent report token"))
    }

    /// Stops the exact current incarnation of `proc`, relaunches it and waits until ready.
    pub(crate) fn replace(&self) -> Result<Value, RunnerError> {
        let mut guard = self.lock()?;
        let replaced = guard
            .as_mut()
            .ok_or_else(|| at("replace")("closed"))
            .and_then(|terminals| self.replace_in(terminals));
        drop(guard);
        replaced
    }

    fn replace_in(&self, terminals: &mut Terminals) -> Result<Value, RunnerError> {
        let id = TerminalId::try_from(PROC_TERMINAL).map_err(at("build the terminal id"))?;
        let record = terminals.get(&self.session, id).map_err(at("read proc"))?;
        let expected = record
            .lifecycle
            .process()
            .ok_or_else(|| at("replace")("no live process"))?
            .clone();
        let killed = terminals
            .kill(
                call(&self.session),
                &KillRequest::immediate(id, expected.clone()),
            )
            .map_err(at("stop the current incarnation"))?;
        terminals
            .recover(call(&self.session), id)
            .map_err(at("relaunch proc"))?;
        self.settle(terminals, id)?;
        Ok(json!({
            "replaced": expected.generation().get().to_string(),
            "exit_observed": killed.exit.is_some(),
        }))
    }

    /// Stops every program, releases the lifecycle service and joins the holder.
    pub(crate) fn stop(&self) -> Result<(), RunnerError> {
        let taken = self.lock()?.take();
        if let Some(mut terminals) = taken {
            terminals
                .program_stop(call(&self.session))
                .map_err(at("stop the programs"))?;
            terminals
                .close()
                .map_err(at("close the lifecycle service"))?;
        }
        self.holder
            .shutdown()
            .map(drop)
            .map_err(at("shut the holder down"))?;
        if self.revocation_failed.load(Ordering::SeqCst) {
            return Err(at("revoke an ended incarnation's token")(
                "the token registry is unusable",
            ));
        }
        Ok(())
    }
}
