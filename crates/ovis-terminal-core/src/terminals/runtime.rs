//! Serialized lifecycle boundary for dedicated blocking control workers.

mod control;
mod inspection;
mod launch;
mod metadata;
mod observe;
mod recovery;
mod stop;

use std::collections::BTreeMap;
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::{
    AccessRequest, Action, Authority, Decision, ResolvedTarget, Session, TargetResource,
};
use ovis_terminal_holder::protocol::{
    CoreBuildId, HOLDER_PROTOCOL_GENERATION, Hello, PreviousCoreState,
};
use ovis_terminal_holder::unix::{CoreLink, Holder};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_store::Store;

use super::{LifecycleEvent, Producer, Record, TerminalError};
use crate::events::Coordinator;
use crate::ops::{Context, Operations, RunningOperation};
use crate::registry::{RegistryIdentities, RuntimeIdentity, TerminalId};

/// The owning holder, authenticated authority and registry runtime identity.
#[derive(Debug)]
pub struct Host {
    identity: RuntimeIdentity,
    authority: Arc<Authority<RegistryIdentities>>,
    holder: Holder,
}

impl Host {
    /// Attaches lifecycle operations to existing runtime dependencies.
    pub const fn new(
        identity: RuntimeIdentity,
        authority: Arc<Authority<RegistryIdentities>>,
        holder: Holder,
    ) -> Self {
        Self {
            identity,
            authority,
            holder,
        }
    }
}

/// An authenticated mutation and its deduplicated operation identity.
#[derive(Debug)]
pub struct Call<'a> {
    session: &'a Session,
    op_id: OpId,
}

impl<'a> Call<'a> {
    /// Binds the request to its authenticated session.
    pub const fn new(session: &'a Session, op_id: OpId) -> Self {
        Self { session, op_id }
    }
}

#[derive(Debug)]
struct Pending {
    permit: RunningOperation,
    context: Context,
    intent: Intent,
}

#[derive(Debug)]
enum Intent {
    Launch,
    Kill(super::KillReceipt),
}

#[derive(Debug)]
enum Incoming {
    Holder(ovis_terminal_holder::protocol::Message),
    Rejected(TerminalId, TerminalError),
    Disconnected(ovis_terminal_holder::unix::RecvError),
}

/// Terminal registry operations serialized by their owning control actor.
///
/// Call from a dedicated blocking worker. Spawn workers are joined by `close`;
/// dropping a control connection does not shut down the separately owned holder.
#[derive(Debug)]
pub struct Terminals {
    core: Coordinator<LifecycleEvent, Producer>,
    host: Host,
    link: CoreLink,
    pending: BTreeMap<TerminalId, Pending>,
    workers: Vec<JoinHandle<()>>,
    messages: mpsc::Receiver<Incoming>,
    sender: mpsc::SyncSender<Incoming>,
    bridge: JoinHandle<()>,
    control: super::ControlState,
}

impl Terminals {
    /// Opens lifecycle ownership over the supplied durable store and live holder.
    ///
    /// # Errors
    ///
    /// Returns holder adoption or committed-state decoding failures.
    pub fn new(store: Store, host: Host) -> Result<Self, TerminalError> {
        let link = host.holder.adopt(&Hello {
            generation: HOLDER_PROTOCOL_GENERATION,
            core_build: CoreBuildId::new(env!("CARGO_PKG_VERSION").into()),
            previous: PreviousCoreState::Crash,
        })?;
        let core = Coordinator::new(
            store,
            host.identity,
            Producer::new(host.identity.runtime_id()),
        );
        let (sender, messages) = mpsc::sync_channel(128);
        let reader = link.clone();
        let delivery = sender.clone();
        let bridge = std::thread::Builder::new()
            .name("ovis-lifecycle-events".into())
            .spawn(move || {
                loop {
                    match reader.recv() {
                        Ok(message) => {
                            if delivery.send(Incoming::Holder(message)).is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            match delivery.send(Incoming::Disconnected(error)) {
                                Ok(()) | Err(_) => {}
                            }
                            break;
                        }
                    }
                }
            })
            .map_err(TerminalError::Worker)?;
        let mut runtime = Self {
            core,
            host,
            link,
            pending: BTreeMap::new(),
            workers: Vec::new(),
            messages,
            sender,
            bridge,
            control: super::ControlState::Running,
        };
        if let Err(error) = runtime.receive(std::time::Duration::from_secs(10)) {
            runtime.close()?;
            return Err(error);
        }
        Ok(runtime)
    }

    /// Exposes the typed coordinator, never its writer, for subscriptions and snapshots.
    pub const fn coordinator(&self) -> &Coordinator<LifecycleEvent, Producer> {
        &self.core
    }

    /// Reads all terminal records without touching a PTY.
    ///
    /// # Errors
    ///
    /// Returns authority or pinned snapshot failures.
    pub fn list(&self, session: &Session) -> Result<Vec<Record>, TerminalError> {
        self.host.authority.authorize(
            session,
            &AccessRequest::new(Action::Read, self.machine_target(), now()?),
        )?;
        Ok(self.core.read_snapshot()?.sections.terminals)
    }

    /// Reads one explicit stable terminal identity.
    ///
    /// # Errors
    ///
    /// Returns `not_found`, authority or snapshot failures; never falls back to focus.
    pub fn get(&self, session: &Session, id: TerminalId) -> Result<Record, TerminalError> {
        let record = self.record(id)?;
        self.authorize(session, (&record, Action::Read))?;
        Ok(record)
    }

    fn record(&self, id: TerminalId) -> Result<Record, TerminalError> {
        self.core
            .read_snapshot()?
            .sections
            .terminals
            .into_iter()
            .find(|record| record.id() == id)
            .ok_or(TerminalError::NotFound)
    }

    const fn machine_target(&self) -> ResolvedTarget<RegistryIdentities> {
        ResolvedTarget::new(self.host.identity.machine_id(), TargetResource::Machine)
    }

    const fn ensure_running(&self) -> Result<(), TerminalError> {
        match self.control {
            super::ControlState::Running => Ok(()),
            super::ControlState::Stopped | super::ControlState::Handover => {
                Err(TerminalError::ControlStopped)
            }
        }
    }

    /// Reads this principal's retained operation without changing process state.
    ///
    /// # Errors
    ///
    /// Returns authority, missing-operation or durable inspection failures.
    pub fn operation(
        &self,
        session: &Session,
        op_id: &OpId,
    ) -> Result<ovis_terminal_protocol::schema::methods::server::OpRecord, TerminalError> {
        self.host.authority.authorize(
            session,
            &AccessRequest::new(Action::Read, self.machine_target(), now()?),
        )?;
        Ok(self.operations(session)?.get(op_id)?)
    }

    const fn target(&self, record: &Record) -> ResolvedTarget<RegistryIdentities> {
        ResolvedTarget::new(
            self.host.identity.machine_id(),
            TargetResource::Terminal {
                placement: None,
                terminal: record.id(),
                incarnation: record.lifecycle.generation(),
            },
        )
    }

    fn authorize(
        &self,
        session: &Session,
        (record, action): (&Record, Action),
    ) -> Result<Decision<RegistryIdentities>, TerminalError> {
        Ok(self.host.authority.authorize(
            session,
            &AccessRequest::new(action, self.target(record), now()?),
        )?)
    }

    fn operations(
        &self,
        session: &Session,
    ) -> Result<Operations<'_, LifecycleEvent, Producer>, TerminalError> {
        Ok(Operations::new(
            &self.core,
            Context::new(session.principal().clone(), self.host.identity, now()?),
        ))
    }

    /// Releases control ownership and joins launch workers without killing any program.
    ///
    /// # Errors
    ///
    /// Returns worker or durable-store shutdown failures.
    pub fn close(self) -> Result<(), TerminalError> {
        drop(self.messages);
        self.link.release();
        for worker in self.workers {
            worker.join().map_err(|_| TerminalError::WorkerPanicked)?;
        }
        self.bridge
            .join()
            .map_err(|_| TerminalError::WorkerPanicked)?;
        self.core.close()?;
        Ok(())
    }
}

fn now() -> Result<TimestampMs, crate::ops::OperationError> {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    Ok(TimestampMs::new(
        u64::try_from(millis).map_err(|_| crate::ops::OperationError::InvalidParams)?,
    ))
}

/// Each operation unit emits its operation event followed by one lifecycle event.
fn stamp_operation(
    record: &mut Record,
    snapshot: &ovis_terminal_store::Snapshot,
) -> Result<(), crate::ops::OperationError> {
    record.revision = ovis_terminal_protocol::schema::scalar::Revision::new(
        snapshot
            .revision()
            .get()
            .checked_add(2)
            .ok_or(crate::events::EventError::LimitExceeded)?,
    );
    Ok(())
}
