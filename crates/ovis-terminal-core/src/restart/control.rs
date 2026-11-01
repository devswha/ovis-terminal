//! The replaceable owner of the terminal lifecycle actor.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};

use ovis_terminal_authz::{Authority, Session};
use ovis_terminal_holder::unix::Holder;
use ovis_terminal_protocol::domain::restart::{RestartBoundary, RestartPhase};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::server::StopScope;
use ovis_terminal_store::{Store, StoreOptions};

use super::{ArmedWaits, Boot, IntentFile, RestartError, StopReceipt};
use crate::registry::{RegistryIdentities, RuntimeIdentity};
use crate::terminals::{Call, ControlReceipt, Host, TerminalError, Terminals};

/// Where the lifecycle state lives, the holder that owns the programs and the
/// runtime's own administrative session that drives handover and adoption.
#[derive(Debug)]
pub struct ControlPlan {
    state: PathBuf,
    holder: Holder,
    authority: Arc<Authority<RegistryIdentities>>,
    session: Session,
}

impl ControlPlan {
    /// Keeps state below `state`, owns programs through `holder` and acts as `session`.
    #[must_use]
    pub fn new(
        state: PathBuf,
        (holder, authority): (Holder, Arc<Authority<RegistryIdentities>>),
        session: Session,
    ) -> Self {
        Self {
            state,
            holder,
            authority,
            session,
        }
    }

    pub(super) fn intent(&self) -> IntentFile {
        IntentFile::new(&self.state)
    }

    pub(super) const fn session(&self) -> &Session {
        &self.session
    }

    /// Adopts the holder as the control of `identity`.
    pub(super) fn boot(&self, identity: RuntimeIdentity) -> Result<Terminals, RestartError> {
        let store = Store::open(&self.state.join("terminals"), StoreOptions::default())?;
        let host = Host::new(identity, Arc::clone(&self.authority), self.holder.clone());
        Ok(Terminals::new(store, host)?)
    }
}

#[derive(Debug)]
pub(super) struct Inner {
    pub(super) terminals: Option<Terminals>,
    pub(super) identity: RuntimeIdentity,
    pub(super) plan: ControlPlan,
}

/// The control of one runtime lifetime across planned restarts.
#[derive(Debug)]
pub struct Control {
    pub(super) inner: Mutex<Inner>,
    pub(super) phase: Mutex<RestartPhase>,
    pub(super) waits: ArmedWaits,
    listeners: Mutex<Vec<SyncSender<RestartBoundary>>>,
    #[cfg(test)]
    pub(super) crash_after: Mutex<Option<RestartPhase>>,
}

impl Control {
    /// Opens control of `identity`; a pending restart intent is finished first.
    ///
    /// With an intent on disk, the successor takes the boot identity the intent names and
    /// the lifetime it recorded, and reports [`Boot::Planned`] with the receipt of the
    /// finished restart; without one it reports [`Boot::Unplanned`].
    ///
    /// # Errors
    ///
    /// `host_rebooted` or `lifetime_mismatch` for an intent this runtime cannot finish,
    /// and typed store, holder or ledger failures.
    pub fn open(
        plan: ControlPlan,
        identity: RuntimeIdentity,
    ) -> Result<(Self, Boot), RestartError> {
        let file = plan.intent();
        let pending = file.load()?;
        let identity = match &pending {
            Some(intent) => super::sequence::successor(intent, identity)?,
            None => identity,
        };
        let terminals = plan.boot(identity)?;
        let control = Self {
            inner: Mutex::new(Inner {
                terminals: Some(terminals),
                identity,
                plan,
            }),
            phase: Mutex::new(RestartPhase::Running),
            waits: ArmedWaits::new(),
            listeners: Mutex::new(Vec::new()),
            #[cfg(test)]
            crash_after: Mutex::new(None),
        };
        let boot = match pending {
            Some(intent) => Boot::Planned(Box::new(control.finish(&intent, true)?)),
            None => Boot::Unplanned,
        };
        Ok((control, boot))
    }

    /// The identity control serves under now.
    ///
    /// # Errors
    ///
    /// [`RestartError::Poisoned`] when a control thread panicked.
    pub fn identity(&self) -> Result<RuntimeIdentity, RestartError> {
        Ok(self.lock()?.identity)
    }

    /// The restart phase; anything but `running` refuses new work.
    ///
    /// # Errors
    ///
    /// [`RestartError::Poisoned`] when a control thread panicked.
    pub fn phase(&self) -> Result<RestartPhase, RestartError> {
        self.phase
            .lock()
            .map(|phase| *phase)
            .map_err(|_| RestartError::Poisoned)
    }

    /// Waits of the current boot; a restart ends each one `cancelled`.
    pub const fn waits(&self) -> &ArmedWaits {
        &self.waits
    }

    /// Restart boundaries announced from now on, queued up to `capacity`.
    ///
    /// A full queue holds the restart at that boundary until the listener takes it; with
    /// `capacity` 0 the restart waits at every boundary, so an owner of attach streams can
    /// end them with `server_restart` before ownership is handed over. A dropped receiver
    /// is forgotten.
    ///
    /// # Errors
    ///
    /// [`RestartError::Poisoned`] when a control thread panicked.
    pub fn boundaries(&self, capacity: usize) -> Result<Receiver<RestartBoundary>, RestartError> {
        let (sender, receiver) = sync_channel(capacity);
        self.listeners
            .lock()
            .map_err(|_| RestartError::Poisoned)?
            .push(sender);
        Ok(receiver)
    }

    /// Runs `work` on the lifecycle actor as the runtime's own session.
    ///
    /// # Errors
    ///
    /// `restarting` while readiness is withdrawn, `control_stopped` after a stop, or the
    /// actor's own failure.
    pub fn with_terminals<R>(
        &self,
        work: impl FnOnce(&mut Terminals, &Session) -> Result<R, TerminalError>,
    ) -> Result<R, RestartError> {
        self.ensure_ready()?;
        let mut inner = self.lock()?;
        // A restart may have taken the lock after the check above and released the old
        // lifecycle meanwhile: that is `restarting`, not a stopped control.
        self.ensure_ready()?;
        let Inner {
            terminals, plan, ..
        } = &mut *inner;
        let terminals = terminals.as_mut().ok_or(RestartError::ControlStopped)?;
        let result = work(terminals, plan.session());
        drop(inner);
        Ok(result?)
    }

    /// Stops control with an explicit scope; `control_only` signals no program.
    ///
    /// # Errors
    ///
    /// `outcome_unknown` when programs were signalled without observed exits (control
    /// stays up), `restarting`, `control_stopped` or the actor's failure.
    pub fn stop(&self, op_id: OpId, scope: StopScope) -> Result<StopReceipt, RestartError> {
        self.ensure_ready()?;
        let mut inner = self.lock()?;
        self.ensure_ready()?;
        let from = inner.identity.server_boot_id();
        let Inner {
            terminals, plan, ..
        } = &mut *inner;
        let actor = terminals.as_mut().ok_or(RestartError::ControlStopped)?;
        let ended = match scope {
            StopScope::ControlOnly => Vec::new(),
            StopScope::ControlAndPrograms => {
                let call = Call::new(plan.session(), OpId::new(uuid::Uuid::now_v7().to_string()));
                match actor.program_stop(call)? {
                    ControlReceipt::ProgramStop { ended, unknown, .. } if unknown.is_empty() => {
                        ended
                    }
                    ControlReceipt::ProgramStop { ended, unknown, .. } => {
                        return Err(RestartError::StopUnproven { ended, unknown });
                    }
                    _ => return Err(TerminalError::InvalidParams.into()),
                }
            }
            _ => return Err(TerminalError::InvalidParams.into()),
        };
        let ControlReceipt::Stopped { preserved, pending } =
            actor.control_stop(Call::new(plan.session(), op_id))?
        else {
            return Err(TerminalError::InvalidParams.into());
        };
        let closing = terminals.take();
        drop(inner);
        if let Some(actor) = closing {
            actor.close()?;
        }
        Ok(StopReceipt::new(from, (preserved, ended), pending))
    }

    /// Releases control without signalling any program.
    ///
    /// # Errors
    ///
    /// Worker or store shutdown failures.
    pub fn close(self) -> Result<(), RestartError> {
        let inner = self
            .inner
            .into_inner()
            .map_err(|_| RestartError::Poisoned)?;
        if let Some(terminals) = inner.terminals {
            terminals.close()?;
        }
        Ok(())
    }

    /// Simulates the control process dying right after the restart crossed `phase`.
    #[cfg(test)]
    pub(super) fn crash_point(&self, phase: RestartPhase) -> Result<(), RestartError> {
        let crash = *self
            .crash_after
            .lock()
            .map_err(|_| RestartError::Poisoned)?;
        match crash {
            Some(crash) if crash == phase => Err(RestartError::Crashed { phase }),
            _ => Ok(()),
        }
    }

    pub(super) fn lock(&self) -> Result<MutexGuard<'_, Inner>, RestartError> {
        self.inner.lock().map_err(|_| RestartError::Poisoned)
    }

    pub(super) fn set(&self, phase: RestartPhase) -> Result<(), RestartError> {
        *self.phase.lock().map_err(|_| RestartError::Poisoned)? = phase;
        Ok(())
    }

    /// Refuses with `restarting` unless control is outside every restart boundary.
    ///
    /// # Errors
    ///
    /// [`RestartError::Restarting`] with the phase reached, or `restart_poisoned`.
    pub fn ensure_ready(&self) -> Result<(), RestartError> {
        match self.phase()? {
            RestartPhase::Running => Ok(()),
            phase => Err(RestartError::Restarting { phase }),
        }
    }

    pub(super) fn announce(&self, boundary: &RestartBoundary) -> Result<(), RestartError> {
        self.listeners
            .lock()
            .map_err(|_| RestartError::Poisoned)?
            .retain(|listener| listener.send(boundary.clone()).is_ok());
        Ok(())
    }
}
