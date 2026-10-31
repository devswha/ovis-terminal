//! `agent.register`, `agent.get`, `status.report`, `status.get` and `status.clear` (plan todo 33).
//!
//! Each method resolves its explicit target through the owning terminal directory, is
//! authorized for its principal, and runs its domain step inside one commit unit; the
//! process token is checked there against the terminal's current program.

use std::fmt::Debug;
use std::sync::Arc;

use ovis_terminal_agent::status::{AgentStatusEvent, Clear, Register, Report, StatusService};
use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_core::registry::{
    MachineId, RegistryIdentities, TargetResolutionError, TerminalGeneration, TerminalId,
};
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::methods::agent::{AgentRegisterParams, AgentTargetParams};
use ovis_terminal_protocol::schema::methods::status::{
    StatusClearParams, StatusReportParams, StatusTargetParams,
};
use ovis_terminal_protocol::schema::scalar::ReportSeq;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::dispatch::execution::now;
use crate::dispatch::{Access, Completed, DispatchError, Execution, Method};

/// A terminal an explicit target resolved to, with its current generation.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct ResolvedTerminal {
    /// Machine owning the terminal.
    pub machine: MachineId,
    /// The terminal.
    pub terminal: TerminalId,
    /// Its current generation.
    pub generation: TerminalGeneration,
}

impl ResolvedTerminal {
    /// Names generation `generation` of `terminal` on `machine`.
    #[must_use]
    pub const fn new(
        machine: MachineId,
        terminal: TerminalId,
        generation: TerminalGeneration,
    ) -> Self {
        Self {
            machine,
            terminal,
            generation,
        }
    }
}

/// The registry that owns terminal identities; it never selects a focused terminal.
pub trait TerminalDirectory: Debug + Send + Sync + 'static {
    /// Resolves one explicit target.
    ///
    /// # Errors
    ///
    /// Returns `not_found`, `ambiguous_target` or another typed resolution failure.
    fn resolve(&self, target: &Target) -> Result<ResolvedTerminal, TargetResolutionError>;
}

impl<T: TerminalDirectory> TerminalDirectory for Arc<T> {
    fn resolve(&self, target: &Target) -> Result<ResolvedTerminal, TargetResolutionError> {
        T::resolve(self, target)
    }
}

/// Shared state of the agent status methods.
#[derive(Debug)]
pub struct AgentStatusLane<T> {
    terminals: T,
    service: Arc<StatusService>,
}

impl<T: TerminalDirectory> AgentStatusLane<T> {
    /// Serves `service` for the terminals `terminals` resolves.
    pub fn new(terminals: T, service: Arc<StatusService>) -> Arc<Self> {
        Arc::new(Self { terminals, service })
    }

    fn access(&self, target: &Target, action: Action) -> Result<Access, DispatchError> {
        terminal_access(&self.terminals, target, action)
    }
}

/// The access an explicit terminal target requests, resolved by `terminals`.
pub(super) fn terminal_access<T: TerminalDirectory>(
    terminals: &T,
    target: &Target,
    action: Action,
) -> Result<Access, DispatchError> {
    let resolved = terminals.resolve(target)?;
    Ok(Access::new(
        action,
        ResolvedTarget::new(
            resolved.machine,
            TargetResource::Terminal {
                placement: None,
                terminal: resolved.terminal,
                incarnation: resolved.generation,
            },
        ),
    ))
}

/// The terminal an execution's resolved target names.
pub(super) const fn terminal<D, P>(
    execution: &Execution<D, P>,
) -> Result<TerminalId, DispatchError> {
    match execution.target().resource() {
        TargetResource::<RegistryIdentities>::Terminal { terminal, .. } => Ok(*terminal),
        _ => Err(DispatchError::Registration),
    }
}

/// Declares one method of the lane over the lane's shared state.
macro_rules! lane_method {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug)]
        pub struct $name<T>(Arc<AgentStatusLane<T>>);

        impl<T> $name<T> {
            /// The method over `lane`'s shared state.
            #[must_use]
            pub fn new(lane: &Arc<AgentStatusLane<T>>) -> Self {
                Self(Arc::clone(lane))
            }
        }
    };
}

lane_method!(
    /// `agent.register`: any label, bound to the owning program's incarnation.
    RegisterAgent
);
lane_method!(
    /// `agent.get`: the binding, or `agent: null` for a plain program.
    GetAgent
);
lane_method!(
    /// `status.report`: a sequenced report by the owning process.
    ReportStatus
);
lane_method!(
    /// `status.get`: declared status with turn and finish facts.
    GetStatus
);
lane_method!(
    /// `status.clear`: removes the root or one child record.
    ClearStatus
);

/// Implements [`Method`] for one lane method over every compatible event domain.
macro_rules! lane_impl {
    ($name:ident, $method:literal, $params:ty, $action:expr, |$lane:ident, $params_var:ident, $terminal:ident| $build:expr) => {
        impl<T, D, P> Method<D, P> for $name<T>
        where
            T: TerminalDirectory,
            D: EventData
                + From<OperationEvent>
                + From<AgentStatusEvent>
                + Clone
                + Serialize
                + DeserializeOwned
                + Send
                + 'static,
            P: SnapshotProducer + Send + Sync + 'static,
            P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
        {
            const NAME: &'static str = $method;
            type Params = $params;

            fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
                self.0.access(&params.target, $action)
            }

            async fn run(
                &self,
                $params_var: Self::Params,
                execution: Execution<D, P>,
            ) -> Result<Completed, DispatchError> {
                let $terminal = terminal(&execution)?;
                let $lane = Arc::clone(&self.0.service);
                let build = $build;
                execution.commit_unit(build).await
            }
        }
    };
}

lane_impl!(
    RegisterAgent,
    "agent.register",
    AgentRegisterParams,
    Action::Report,
    |lane, params, terminal| {
        let mut register = Register::new(terminal, params.process_token, params.label);
        register.session = params.session;
        move |snapshot: &_| lane.register(snapshot, register)
    }
);
lane_impl!(
    GetAgent,
    "agent.get",
    AgentTargetParams,
    Action::Read,
    |lane, params, terminal| {
        drop(params);
        move |snapshot: &_| lane.agent(snapshot, terminal)
    }
);
lane_impl!(
    GetStatus,
    "status.get",
    StatusTargetParams,
    Action::Read,
    |lane, params, terminal| {
        drop(params);
        move |snapshot: &_| lane.status(snapshot, terminal)
    }
);
lane_impl!(
    ClearStatus,
    "status.clear",
    StatusClearParams,
    Action::Report,
    |lane, params, terminal| {
        let clear = Clear::new(terminal, params.process_token, params.id);
        move |snapshot: &_| lane.clear(snapshot, &clear)
    }
);
lane_impl!(
    ReportStatus,
    "status.report",
    StatusReportParams,
    Action::Report,
    |lane, params, terminal| {
        let mut report = Report::new(terminal, params.process_token, params.record);
        report.seq = ReportSeq::new(params.seq.get());
        report.at = now()?;
        move |snapshot: &_| lane.report(snapshot, report)
    }
);
