//! The holder: owns every PTY and child, drains output, and reports to one core.

mod admission;
mod control;
mod ending;
mod state;
mod threads;
mod workers;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use self::state::{HolderState, LiveTerminal, Terminal};
use super::config::{HolderConfig, ProcessEnvironment};
use super::error::HolderError;
use super::input::InputQueue;
use super::join_report::{JoinOutcome, WorkerRole};
use super::launch::{Grid, LaunchRequest, LaunchSpec, launch};
use super::launch_error::LaunchError;
use super::link::{CoreLink, Outbox};
use super::tail::RetainedTail;
use crate::protocol::{
    HeldTerminal, Hello, Message, Spawn, SpawnFailed, SpawnFailureKind, SpawnFailureReason,
    Spawned, TerminalId, TerminalIncarnation, negotiate,
};

pub(super) struct Shared {
    config: HolderConfig,
    state: Mutex<HolderState>,
    spawning: Mutex<()>,
    workers: Mutex<Workers>,
}

#[derive(Default)]
struct Workers {
    running: Vec<(TerminalId, WorkerRole, JoinHandle<()>)>,
    outcomes: BTreeMap<(TerminalId, WorkerRole), JoinOutcome>,
    started: BTreeSet<TerminalId>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, HolderState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `operation` with the state locked for exactly its duration.
    fn with_state<T>(&self, operation: impl FnOnce(&mut HolderState) -> T) -> T {
        operation(&mut self.lock())
    }
}

/// A holder shared by every terminal of one session and build.
///
/// Cloning yields another handle to the same holder.
#[derive(Clone)]
pub struct Holder {
    shared: Arc<Shared>,
}

impl fmt::Debug for Holder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Holder")
            .field("descriptor", &self.shared.config.descriptor)
            .finish_non_exhaustive()
    }
}

impl Holder {
    /// Handles to the shared holder state; each live worker thread owns one.
    #[cfg(test)]
    pub(super) fn handle_count(&self) -> usize {
        Arc::strong_count(&self.shared)
    }

    /// Creates a holder with no terminals and no adopted core.
    #[must_use]
    pub fn new(config: HolderConfig) -> Self {
        Self {
            shared: Arc::new(Shared {
                config,
                state: Mutex::new(HolderState::default()),
                spawning: Mutex::new(()),
                workers: Mutex::new(Workers::default()),
            }),
        }
    }

    /// Adopts the holder for a core, replacing any previous core's link.
    ///
    /// The link's first message is the welcome listing every retained terminal.
    ///
    /// # Errors
    ///
    /// Returns [`HolderError::Adoption`] for an unsupported generation, before
    /// any ownership changes.
    pub fn adopt(&self, hello: &Hello) -> Result<CoreLink, HolderError> {
        let mut state = self.shared.lock();
        let table = state.adoption_table()?;
        let welcome = negotiate(hello, &self.shared.config.descriptor, &table)?;
        let outbox = Arc::new(Outbox::new(self.shared.config.limits.link_bytes));
        outbox
            .push(Message::Welcome(welcome))
            .map_err(|reason| HolderError::LinkClosed { reason })?;
        state.attach(Arc::clone(&outbox));
        drop(state);
        Ok(CoreLink::new(outbox))
    }

    /// Spawns a program on a new PTY in a terminal identity the holder does not hold.
    ///
    /// `Spawned` (or `SpawnFailed`) is also published to the adopted core,
    /// ahead of any output of the new process.
    ///
    /// # Errors
    ///
    /// Returns [`HolderError::Launch`] when the program could not start,
    /// [`HolderError::DuplicateTerminal`], [`HolderError::Draining`], or
    /// [`HolderError::WorkerUnavailable`] when its threads could not start.
    pub fn spawn(&self, request: LaunchRequest) -> Result<Spawned, HolderError> {
        let _serialized = self
            .shared
            .spawning
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let process_incarnation = {
            let mut state = self.shared.lock();
            if state.terminals.contains_key(&request.terminal_id) {
                return Err(HolderError::DuplicateTerminal {
                    terminal_id: request.terminal_id,
                });
            }
            if state.draining {
                let refusal = Refusal {
                    kind: SpawnFailureKind::Unavailable,
                    reason: SpawnFailureReason::HolderDraining,
                    hint: HolderError::Draining.hint(),
                    os_error: None,
                };
                state.publish(refusal.message(&request.terminal_id, request.terminal_incarnation));
                return Err(HolderError::Draining);
            }
            state.allocate_process()?
        };
        let process = match launch(&request.spec) {
            Ok(process) => process,
            Err(error) => {
                let refusal = Refusal::launch(&error);
                let message = refusal.message(&request.terminal_id, request.terminal_incarnation);
                self.shared.lock().publish(message);
                return Err(error.into());
            }
        };
        let spawned = Spawned {
            terminal_id: request.terminal_id.clone(),
            terminal_incarnation: request.terminal_incarnation,
            process_incarnation,
            pid: process.pid.get(),
            start_token: process.start_token.clone(),
        };
        let input = Arc::new(InputQueue::new(self.shared.config.limits.input));
        let pid = process.pid;
        let parts = workers::Parts::split(process, Arc::clone(&input));
        // Workers start under the state lock, so none can publish before `Spawned`.
        let mut state = self.shared.lock();
        state.terminals.insert(
            request.terminal_id.clone(),
            Terminal {
                held: HeldTerminal {
                    terminal_id: request.terminal_id.clone(),
                    terminal_incarnation: request.terminal_incarnation,
                    process_incarnation: Some(process_incarnation),
                    pid: Some(pid.get()),
                    start_token: Some(spawned.start_token.clone()),
                    output_offset: 0,
                    input_sequence: 0,
                    exit: None,
                },
                tail: RetainedTail::new(self.shared.config.limits.tail_bytes),
                live: Some(LiveTerminal {
                    master: parts.master,
                    input,
                    pid,
                }),
                geometry_epoch: None,
                admitted_sequence: None,
                io_errors: Vec::new(),
                first_io_failure: None,
            },
        );
        if let Err(source) = workers::start(&self.shared, &request.terminal_id, parts.threads) {
            let cleanup = workers::abandon(&mut state, &request.terminal_id, pid).err();
            let refusal = Refusal {
                kind: SpawnFailureKind::LimitExceeded,
                reason: SpawnFailureReason::WorkerUnavailable,
                hint: "Free thread or memory resources and relaunch.",
                os_error: source.raw_os_error(),
            };
            state.publish(refusal.message(&request.terminal_id, request.terminal_incarnation));
            return Err(HolderError::WorkerUnavailable { source, cleanup });
        }
        state.publish(Message::Spawned(spawned.clone()));
        drop(state);
        Ok(spawned)
    }

    /// Spawns a wire `Spawn`, which carries the complete per-launch argv, working
    /// directory, environment and grid.
    ///
    /// # Errors
    ///
    /// The same as [`Holder::spawn`], including [`HolderError::Launch`] for a
    /// zero grid dimension.
    pub fn spawn_wire(&self, spawn: &Spawn) -> Result<Spawned, HolderError> {
        let (cols, rows) = spawn.geometry;
        let grid = match Grid::new(cols, rows) {
            Ok(grid) => grid,
            Err(error) => {
                let refusal = Refusal::launch(&error);
                let message = refusal.message(&spawn.terminal_id, spawn.terminal_incarnation);
                self.shared.lock().publish(message);
                return Err(error.into());
            }
        };
        let argv = spawn.argv.iter().map(OsString::from).collect();
        let vars = spawn
            .env
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect();
        let environment = ProcessEnvironment::new(spawn.cwd.clone(), vars);
        self.spawn(LaunchRequest::new(
            spawn.terminal_id.clone(),
            spawn.terminal_incarnation,
            LaunchSpec::new(argv, environment, grid),
        ))
    }
}

/// What a refused spawn reports on the wire; no terminal record is kept.
struct Refusal {
    kind: SpawnFailureKind,
    reason: SpawnFailureReason,
    hint: &'static str,
    os_error: Option<i32>,
}

impl Refusal {
    fn launch(error: &LaunchError) -> Self {
        Self {
            kind: error.failure_kind(),
            reason: error.reason(),
            hint: error.hint(),
            os_error: error.os_error(),
        }
    }

    fn message(
        self,
        terminal_id: &TerminalId,
        terminal_incarnation: TerminalIncarnation,
    ) -> Message {
        Message::SpawnFailed(SpawnFailed {
            terminal_id: terminal_id.clone(),
            terminal_incarnation,
            kind: self.kind,
            reason: self.reason,
            hint: self.hint.to_owned(),
            os_error: self.os_error,
        })
    }
}
