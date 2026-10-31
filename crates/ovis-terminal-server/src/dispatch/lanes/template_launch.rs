//! `terminal.run` for argv and template programs, `terminal.first_message`,
//! `template.list` and `template.get` (plan todo 35).
//!
//! A template resolves to the same launch plan as a direct argument vector and both
//! start through one [`Launcher`]. A first message is queued under the launch
//! operation and the requesting principal; its declared-state wait is armed on the
//! reserved terminal's wait hub before the program starts, and a worker releases it
//! once through the write boundary or settles it `expired` or `cancelled`. Dropping
//! the lane cancels every pending wait and joins its workers.

mod reads;
mod run;

use std::collections::VecDeque;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use ovis_terminal_authz::{Action, Authority, Decision, ResolvedTarget, TargetResource};
use ovis_terminal_core::input::encoding::ModeSnapshot;
use ovis_terminal_core::launch::FirstMessages;
use ovis_terminal_core::leases::LeaseView;
use ovis_terminal_core::registry::{MachineId, ProcessGeneration, RegistryIdentities, TerminalId};
use ovis_terminal_core::waits::{CancelScope, Waits};
use ovis_terminal_protocol::domain::template_launch::LaunchedTerminal;
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::terminal::TerminalRunParams;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_services::templates::{LaunchPlan, TemplateCatalog};
pub use reads::{GetFirstMessage, GetTemplate, ListTemplates};
pub use run::RunTerminal;

use crate::dispatch::{Access, DispatchError};

type Registry = RegistryIdentities;

/// Input decisions kept between admission and execution of a launch.
const DECISIONS: usize = 256;

/// One launch as the launcher receives it: the resolved plan and the request.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Launch<'a> {
    /// Literal argv, environment, readiness states and first message.
    pub plan: &'a LaunchPlan,
    /// The request's metadata: geometry, cwd, name, labels, parent, policy.
    pub params: &'a TerminalRunParams,
}

/// A terminal identity and program incarnation reserved before anything runs.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Reservation {
    /// Reserved terminal.
    pub terminal: TerminalId,
    /// Program incarnation the start will create.
    pub process: ProcessGeneration,
    /// The incarnation's wait hub, fed by its output, exit and declared status.
    pub waits: Waits,
}

impl Reservation {
    /// Reserves `terminal` running `process`, observed through `waits`.
    #[must_use]
    pub const fn new(terminal: TerminalId, process: ProcessGeneration, waits: Waits) -> Self {
        Self {
            terminal,
            process,
            waits,
        }
    }
}

/// A terminal's state at the moment a first message is written.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Boundary {
    /// Its leases.
    pub leases: LeaseView,
    /// Its input modes.
    pub modes: ModeSnapshot,
    /// The program incarnation running in it.
    pub process: ProcessGeneration,
}

impl Boundary {
    /// The state of a terminal with `leases`, `modes` and `process` running.
    #[must_use]
    pub const fn new(leases: LeaseView, modes: ModeSnapshot, process: ProcessGeneration) -> Self {
        Self {
            leases,
            modes,
            process,
        }
    }
}

/// The launch path every program takes: reserve, start, observe and write.
pub trait Launcher: Debug + Send + Sync + 'static {
    /// Reserves a terminal and its wait hub; nothing runs and nothing feeds the hub yet.
    ///
    /// # Errors
    ///
    /// Typed launch refusals such as `duplicate_name` or `invalid_geometry`.
    fn reserve(&self, launch: Launch<'_>) -> Result<Reservation, DispatchError>;

    /// Starts the reserved program with the plan's literal argv and environment.
    ///
    /// # Errors
    ///
    /// `spawn_failed` and other typed launch failures.
    fn start(
        &self,
        reservation: &Reservation,
        launch: Launch<'_>,
    ) -> Result<LaunchedTerminal, DispatchError>;

    /// The terminal's leases, modes and running incarnation now.
    ///
    /// # Errors
    ///
    /// When the terminal is no longer known.
    fn boundary(&self, terminal: TerminalId) -> Result<Boundary, DispatchError>;

    /// Writes one encoded submission and answers its input sequence.
    ///
    /// # Errors
    ///
    /// When the terminal's writer refuses the bytes.
    fn write(&self, terminal: TerminalId, bytes: &[u8]) -> Result<EventSeq, DispatchError>;
}

#[derive(Debug)]
struct Shared<L> {
    launcher: L,
    book: FirstMessages,
    authority: Arc<Authority<Registry>>,
}

/// Shared state of the template launch methods.
#[derive(Debug)]
pub struct TemplateLaunchLane<L> {
    shared: Arc<Shared<L>>,
    catalog: TemplateCatalog,
    machine: MachineId,
    decisions: Mutex<VecDeque<(OpId, Decision<Registry>)>>,
    scope: CancelScope,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

impl<L: Launcher> TemplateLaunchLane<L> {
    /// Serves templates from `catalog` and launches through `launcher` on `machine`.
    pub fn new(
        (catalog, launcher): (TemplateCatalog, L),
        authority: Arc<Authority<Registry>>,
        machine: MachineId,
    ) -> Arc<Self> {
        Arc::new(Self {
            shared: Arc::new(Shared {
                launcher,
                book: FirstMessages::new(),
                authority,
            }),
            catalog,
            machine,
            decisions: Mutex::new(VecDeque::new()),
            scope: CancelScope::new(),
            workers: Mutex::new(Vec::new()),
        })
    }

    /// The launcher the lane starts programs through.
    pub fn launcher(&self) -> &L {
        &self.shared.launcher
    }

    const fn access(&self, action: Action) -> Access {
        Access::new(
            action,
            ResolvedTarget::new(self.machine, TargetResource::Machine),
        )
    }

    fn keep_decision(&self, op_id: OpId, decision: Decision<Registry>) {
        let mut decisions = self
            .decisions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        decisions.retain(|(kept, _)| kept != &op_id);
        if decisions.len() >= DECISIONS {
            decisions.pop_front();
        }
        decisions.push_back((op_id, decision));
    }

    fn take_decision(&self, op_id: &OpId) -> Option<Decision<Registry>> {
        let mut decisions = self
            .decisions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let index = decisions.iter().position(|(kept, _)| kept == op_id)?;
        decisions.remove(index).map(|(_, decision)| decision)
    }
}

impl<L> Drop for TemplateLaunchLane<L> {
    fn drop(&mut self) {
        self.scope.cancel();
        let workers = std::mem::take(
            self.workers
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner),
        );
        // A worker that panicked left its message unreleased; Drop has no caller to tell.
        workers.into_iter().map(JoinHandle::join).for_each(drop);
    }
}

/// A launch the lane refuses before anything is reserved.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LaunchRefusal {
    /// The launch carries a first message, but its principal may not write input.
    #[error("the launching principal may not write the first message")]
    FirstMessageWithoutInput,
}

impl LaunchRefusal {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::FirstMessageWithoutInput => "forbidden",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::FirstMessageWithoutInput => {
                "launch without a first message, or with a credential that holds input rights"
            }
        }
    }
}

/// A domain refusal carried with its own kind and hint.
fn refusal<E>(kind: &'static str, hint: &'static str, source: E) -> DispatchError
where
    E: std::error::Error + Send + Sync + 'static,
{
    DispatchError::Domain {
        kind,
        hint,
        source: Box::new(source),
    }
}
