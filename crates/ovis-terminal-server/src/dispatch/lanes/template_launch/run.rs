//! `terminal.run`: one launch path for argv and template programs.

use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use ovis_terminal_authz::{AccessRequest, Action, Decision, PrincipalId, Session};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::launch::{
    QueuedMessage, Settlement, Verdict, WriteBoundary, arm, await_ready,
};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_core::registry::{ProcessGeneration, TerminalId};
use ovis_terminal_core::waits::{StateOutcome, Ticket};
use ovis_terminal_protocol::domain::template_launch::{
    FirstMessageReason, FirstMessageRecord, LaunchSource,
};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::TerminalId as WireTerminalId;
use ovis_terminal_protocol::schema::methods::terminal::TerminalRunParams;
use ovis_terminal_protocol::schema::scalar::{EventSeq, Incarnation, TimestampMs};
use ovis_terminal_services::templates::{LaunchPlan, PlannedMessage};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{
    Launch, LaunchRefusal, Launcher, Registry, Reservation, Shared, TemplateLaunchLane, refusal,
};
use crate::dispatch::execution::now;
use crate::dispatch::{Access, Completed, DispatchError, Execution, Method};

/// `terminal.run`: resolves the program, queues the first message, starts the launch.
#[derive(Debug)]
pub struct RunTerminal<L>(Arc<TemplateLaunchLane<L>>);

impl<L> RunTerminal<L> {
    /// The method over `lane`'s shared state.
    #[must_use]
    pub fn new(lane: &Arc<TemplateLaunchLane<L>>) -> Self {
        Self(Arc::clone(lane))
    }
}

/// What the launch effect needs once the operation may commit.
struct Admitted {
    plan: LaunchPlan,
    params: TerminalRunParams,
    launch_op: OpId,
    principal: PrincipalId,
    decision: Option<Decision<Registry>>,
}

impl<L: Launcher> TemplateLaunchLane<L> {
    fn plan(&self, params: &TerminalRunParams) -> Result<LaunchPlan, DispatchError> {
        let source =
            LaunchSource::of(params).map_err(|error| refusal(error.kind(), error.hint(), error))?;
        match source {
            LaunchSource::Argv(argv) => Ok(LaunchPlan::direct(argv, params)),
            LaunchSource::Template {
                name,
                version,
                task,
            } => self
                .catalog
                .get(name, version)
                .map(|template| template.plan(params, task))
                .map_err(|error| refusal(error.kind(), error.hint(), error)),
            _ => Err(DispatchError::Registration),
        }
    }

    /// Reserves, arms readiness, queues the message, starts the program and hands the
    /// wait to a worker; runs inside the operation's commit.
    fn launch(&self, admitted: Admitted) -> Result<serde_json::Value, DispatchError> {
        let Admitted {
            plan,
            params,
            launch_op,
            principal,
            decision,
        } = admitted;
        let launch = Launch {
            plan: &plan,
            params: &params,
        };
        let shared = &self.shared;
        let reservation = shared.launcher.reserve(launch)?;
        let queued = match (&plan.first_message, decision) {
            (Some(message), Some(decision)) => {
                let ticket = arm(&reservation.waits, &plan.readiness, &self.scope)
                    .map_err(|error| refusal(error.kind(), error.hint(), error))?;
                let record = self.queue((launch_op, principal), (&reservation, message))?;
                Some((record, ticket, decision, message.deadline))
            }
            _ => None,
        };
        let launched = shared.launcher.start(&reservation, launch)?;
        let first_message = queued.map(|(record, ticket, decision, deadline)| {
            let worker = Worker {
                shared: Arc::clone(&self.shared),
                launch_op: record.launch_op.clone(),
                reservation: (reservation.terminal, reservation.process),
                decision,
            };
            let wait = Duration::from_millis(deadline.get());
            let handle = std::thread::spawn(move || worker.run(ticket, wait));
            self.workers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(handle);
            record
        });
        Ok(serde_json::to_value(launched.into_result(first_message))?)
    }

    fn queue(
        &self,
        (launch_op, principal): (OpId, PrincipalId),
        (reservation, message): (&Reservation, &PlannedMessage),
    ) -> Result<FirstMessageRecord, DispatchError> {
        let deadline = now()?.get().saturating_add(message.deadline.get());
        let record = FirstMessageRecord::accepted(
            (
                launch_op,
                WireTerminalId::new(reservation.terminal.to_string()),
            ),
            Incarnation::new(reservation.process.get()),
            TimestampMs::new(deadline),
        );
        let queued = QueuedMessage::new(principal, message.text.clone(), record);
        self.shared
            .book
            .accept(queued)
            .map_err(|error| refusal(error.kind(), error.hint(), error))
    }
}

/// The release of one queued message after its readiness wait.
struct Worker<L> {
    shared: Arc<Shared<L>>,
    launch_op: OpId,
    reservation: (TerminalId, ProcessGeneration),
    decision: Decision<Registry>,
}

impl<L: Launcher> Worker<L> {
    fn run(self, ticket: Ticket<StateOutcome>, wait: Duration) {
        let deadline = Instant::now()
            .checked_add(wait)
            .unwrap_or_else(Instant::now);
        let book = &self.shared.book;
        let settled = match await_ready(ticket, deadline) {
            Ok(Verdict::Ready) => book.release(&self.launch_op, |text| self.write(text)),
            Ok(Verdict::Settle(end)) => book.settle(&self.launch_op, end),
            Ok(_) | Err(_) => book.settle(
                &self.launch_op,
                Settlement::Cancelled(FirstMessageReason::RuntimeStopped),
            ),
        };
        // The message was accepted before this worker started and the book evicts only
        // settled messages, so the settled record is always found here.
        drop(settled);
    }

    fn write(&self, text: &str) -> Result<EventSeq, FirstMessageReason> {
        let (terminal, bound) = self.reservation;
        let launcher = &self.shared.launcher;
        let state = launcher
            .boundary(terminal)
            .map_err(|_| FirstMessageReason::LinkLost)?;
        let at = now().map_err(|_| FirstMessageReason::DeliveryFailed)?;
        let boundary = WriteBoundary::new(
            (&self.shared.authority, &self.decision),
            at,
            (&state.leases, (bound, state.process), &state.modes),
        );
        let encoded = boundary.admit(text)?;
        launcher
            .write(terminal, encoded.bytes())
            .map_err(|_| FirstMessageReason::DeliveryFailed)
    }
}

impl<L, D, P> Method<D, P> for RunTerminal<L>
where
    L: Launcher,
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    const NAME: &'static str = "terminal.run";
    type Params = TerminalRunParams;

    fn resolve(&self, params: &Self::Params, session: &Session) -> Result<Access, DispatchError> {
        let lane = &self.0;
        let access = lane.access(Action::Launch);
        let input = AccessRequest::new(Action::Input, access.target.clone(), now()?);
        if let Ok(decision) = lane.shared.authority.authorize(session, &input) {
            lane.keep_decision(params.op_id.clone(), decision);
        }
        Ok(access)
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<D, P>,
    ) -> Result<Completed, DispatchError> {
        let lane = Arc::clone(&self.0);
        let plan = lane.plan(&params)?;
        let decision = lane.take_decision(&params.op_id);
        if plan.first_message.is_some() && decision.is_none() {
            let refused = LaunchRefusal::FirstMessageWithoutInput;
            return Err(refusal(refused.kind(), refused.hint(), refused));
        }
        let admitted = Admitted {
            plan,
            params,
            launch_op: execution.op_id().clone(),
            principal: execution.decision.principal().clone(),
            decision,
        };
        execution.commit(move || lane.launch(admitted)).await
    }
}
