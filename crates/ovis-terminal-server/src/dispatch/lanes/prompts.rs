//! `prompt.open`, `prompt.answer`, `prompt.close`, `prompt.get` and `prompt.list` (plan todo 34).
//!
//! A prompt request that names only a prompt is routed to the prompt's terminal and
//! authorized there: answering needs `prompt_answer` on that terminal, never interact,
//! takeover or a blocked status. The commit unit rechecks the grant, then validates the
//! version, state, liveness and schema and admits typed input, all in one transaction.

use std::fmt::Debug;
use std::sync::Arc;

use ovis_terminal_agent::prompts::{Answer, Close, Open, PromptError, PromptService};
use ovis_terminal_agent::status::AgentStatusEvent;
use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_core::registry::{MachineId, TerminalId};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::ids::{PromptId, TerminalId as WireTerminal};
use ovis_terminal_protocol::schema::methods::status::{
    PromptAnswerParams, PromptCloseParams, PromptGetParams, PromptListParams, PromptListResult,
    PromptOpenParams,
};
use ovis_terminal_store::Snapshot;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::agent_status::{TerminalDirectory, terminal, terminal_access};
use crate::dispatch::{Access, Completed, DispatchError, Execution, Method, Read};

/// Shared state of the prompt methods.
#[derive(Debug)]
pub struct PromptLane<T> {
    terminals: T,
    service: Arc<PromptService>,
    machine: MachineId,
}

impl<T: TerminalDirectory> PromptLane<T> {
    /// Serves `service` for the terminals of `machine` that `terminals` resolves.
    pub fn new(terminals: T, service: Arc<PromptService>, machine: MachineId) -> Arc<Self> {
        Arc::new(Self {
            terminals,
            service,
            machine,
        })
    }

    /// The terminal `prompt` was opened on, authorized for `action`.
    fn prompt_access(&self, prompt: &PromptId, action: Action) -> Result<Access, DispatchError> {
        let routed = self.service.route(prompt).map_err(refused)?;
        let target = Target::TerminalId(WireTerminal::new(routed.to_string()));
        terminal_access(&self.terminals, &target, action)
    }
}

fn refused(error: PromptError) -> DispatchError {
    DispatchError::Domain {
        kind: error.kind(),
        hint: error.hint(),
        source: Box::new(error),
    }
}

fn answered<R: Serialize>(read: Result<R, PromptError>) -> Result<Value, BoundaryError> {
    let domain = |error: &PromptError| BoundaryError::Domain {
        data: Box::new(error.error_data()),
    };
    let value = read.map_err(|error| domain(&error))?;
    serde_json::to_value(value).map_err(|error| domain(&PromptError::Encode(error)))
}

macro_rules! lane_method {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug)]
        pub struct $name<T>(Arc<PromptLane<T>>);

        impl<T> $name<T> {
            /// The method over `lane`'s shared state.
            #[must_use]
            pub fn new(lane: &Arc<PromptLane<T>>) -> Self {
                Self(Arc::clone(lane))
            }
        }
    };
}

lane_method!(
    /// `prompt.open`: publish or republish a prompt as its owning program.
    OpenPrompt
);
lane_method!(
    /// `prompt.answer`: answer one version under the `prompt_answer` permission.
    AnswerPrompt
);
lane_method!(
    /// `prompt.close`: close a prompt as its owning program.
    ClosePrompt
);
lane_method!(
    /// `prompt.get`: one prompt, read without a receipt.
    GetPrompt
);
lane_method!(
    /// `prompt.list`: a terminal's or every terminal's prompts, read without a receipt.
    ListPrompts
);

macro_rules! lane_impl {
    ($name:ident, $method:literal, $params:ty,
     |$lane:ident, $params_var:ident| $resolve:expr,
     |$service:ident, $run_params:ident, $execution:ident| $build:expr) => {
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

            fn resolve(
                &self,
                $params_var: &Self::Params,
                _: &Session,
            ) -> Result<Access, DispatchError> {
                let $lane = &self.0;
                $resolve
            }

            async fn run(
                &self,
                $run_params: Self::Params,
                $execution: Execution<D, P>,
            ) -> Result<Completed, DispatchError> {
                let $service = Arc::clone(&self.0.service);
                let build = $build;
                $execution.commit_unit(build).await
            }
        }
    };
}

lane_impl!(
    OpenPrompt,
    "prompt.open",
    PromptOpenParams,
    |lane, params| terminal_access(&lane.terminals, &params.target, Action::Report),
    |service, params, execution| {
        let mut open = Open::new(terminal(&execution)?, params.process_token, params.prompt);
        open.prompt_id = params.prompt_id;
        move |snapshot: &_| service.open(snapshot, open)
    }
);
lane_impl!(
    AnswerPrompt,
    "prompt.answer",
    PromptAnswerParams,
    |lane, params| lane.prompt_access(&params.prompt_id, Action::PromptAnswer),
    |service, params, execution| {
        let routed = (terminal(&execution)?, execution.op_id().clone());
        let mut answer = Answer::new(routed, (params.prompt_id, params.version), params.answer);
        answer.lease = params.lease_id;
        move |snapshot: &_| service.answer(snapshot, answer)
    }
);
lane_impl!(
    ClosePrompt,
    "prompt.close",
    PromptCloseParams,
    |lane, params| lane.prompt_access(&params.prompt_id, Action::Report),
    |service, params, execution| {
        let owner = (terminal(&execution)?, params.process_token);
        let close = Close::new(owner, params.prompt_id, params.outcome);
        move |snapshot: &_| service.close(snapshot, &close)
    }
);

impl<T: TerminalDirectory> Read for GetPrompt<T> {
    const NAME: &'static str = "prompt.get";
    type Params = PromptGetParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        self.0.prompt_access(&params.prompt_id, Action::Read)
    }

    fn read(&self, params: Self::Params, snapshot: &Snapshot) -> Result<Value, BoundaryError> {
        answered(self.0.service.get(snapshot, &params.prompt_id))
    }
}

impl<T: TerminalDirectory> Read for ListPrompts<T> {
    const NAME: &'static str = "prompt.list";
    type Params = PromptListParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        params.target.as_ref().map_or_else(
            || {
                let machine = ResolvedTarget::new(self.0.machine, TargetResource::Machine);
                Ok(Access::new(Action::Read, machine))
            },
            |target| terminal_access(&self.0.terminals, target, Action::Read),
        )
    }

    fn read(&self, params: Self::Params, snapshot: &Snapshot) -> Result<Value, BoundaryError> {
        let terminal = match &params.target {
            Some(target) => Some(listed(&self.0.terminals, target)?),
            None => None,
        };
        let filter = (terminal, params.states.as_deref());
        answered(
            self.0
                .service
                .list(snapshot, filter)
                .map(PromptListResult::new),
        )
    }
}

/// The terminal a listing names; resolution already succeeded during authorization.
fn listed<T: TerminalDirectory>(
    terminals: &T,
    target: &Target,
) -> Result<TerminalId, BoundaryError> {
    terminals
        .resolve(target)
        .map(|resolved| resolved.terminal)
        .map_err(|error| DispatchError::from(error).boundary())
}
