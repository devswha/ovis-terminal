//! `turn.wait`, `events.wait` and `input.interrupt` on a terminal's wait hub, and the feed
//! that hands each committed status report to that hub.
//!
//! Each wait is armed on the actor under the control's armed waits (a planned restart
//! ends it `cancelled`) and awaited off the actor. Status entries carry no input position,
//! so a report is attributed to the terminal's latest queued input: a report that follows
//! a send belongs to the turn that send started.

use std::time::{Duration, Instant};

use ovis_terminal_authz::Session;
use ovis_terminal_core::input::receipts::Receipt;
use ovis_terminal_core::input::sequencer::Payload;
use ovis_terminal_core::registry::TerminalId;
use ovis_terminal_core::restart::ArmedWait;
use ovis_terminal_core::waits::{
    Observed, StateOutcome, StateWait, StatusFact, StopWatch, Ticket, TurnBinding, TurnOutcome,
    TurnPosition,
};
use ovis_terminal_protocol::domain::restart::InterruptedWait;
use ovis_terminal_protocol::errors::{BoundaryError, OpId};
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::ids::TerminalId as WireTerminalId;
use ovis_terminal_protocol::schema::methods::events::EventsWaitParams;
use ovis_terminal_protocol::schema::methods::input::InputInterruptParams;
use ovis_terminal_protocol::schema::methods::status::TurnWaitParams;
use ovis_terminal_protocol::schema::records::StatusSnapshot;
use ovis_terminal_protocol::schema::scalar::{ByteOffset, DurationMs, EventSeq, OpenEnum};
use serde_json::{Value, json};

use super::actor::Runtime;
use super::error::refusal;
use super::input::{Admission, live_terminal};
use super::reads::wait_refusal;

/// The key an interrupt sends when the request names none.
const INTERRUPT_KEY: &str = "ctrl+c";

/// One armed wait: its ticket, deadline and the guard that keeps it restart-cancellable.
pub(super) type Armed<T> = (Ticket<T>, Instant, ArmedWait);

fn deadline(timeout: DurationMs) -> Result<Instant, BoundaryError> {
    Instant::now()
        .checked_add(Duration::from_millis(timeout.get()))
        .ok_or_else(|| refusal("invalid_params", "the timeout is too large"))
}

/// The published turn record of a turn wait's outcome; a turn that never started is 0.
pub(super) fn turn_record(outcome: &TurnOutcome, send_op: Option<&OpId>) -> Value {
    let (turn, outcome) = match *outcome {
        TurnOutcome::Completed { turn, .. } => (Some(turn), "completed"),
        TurnOutcome::Blocked { turn, .. } => (Some(turn), "blocked"),
        TurnOutcome::Failed { turn, .. } => (Some(turn), "failed"),
        TurnOutcome::Exited { turn, .. } => (turn, "exited"),
        TurnOutcome::Lost { turn } => (turn, "lost"),
        TurnOutcome::Cancelled { turn } => (turn, "cancelled"),
        TurnOutcome::Timeout { turn } => (turn, "timeout"),
        _ => (None, "lost"),
    };
    json!({"turn": turn.map_or_else(|| "0".to_owned(), |turn| turn.get().to_string()),
        "outcome": outcome, "started_seq": null, "ended_seq": null, "send_op": send_op,
        "finish_generation": null, "ended_on": null, "process_incarnation": null,
        "agent_id": null, "session": null})
}

/// The published `events.wait` outcome; a wait a planned restart ended is refused.
pub(super) fn state_outcome(outcome: &StateOutcome) -> Result<&'static str, BoundaryError> {
    Ok(match outcome {
        StateOutcome::AlreadySatisfied { .. } => "already_satisfied",
        StateOutcome::Matched { .. } => "matched",
        StateOutcome::Exited { .. } => "exited",
        StateOutcome::Timeout => "timeout",
        StateOutcome::Cancelled => {
            return Err(refusal(
                "cancelled",
                "a planned restart ended the wait; wait again on the successor",
            ));
        }
        _ => "lost",
    })
}

impl Runtime {
    /// Hands a committed status report to `terminal`'s wait hub.
    pub(super) fn feed_status(
        &mut self,
        terminal: TerminalId,
        (snapshot, seq): (&StatusSnapshot, EventSeq),
    ) {
        let Some(live) = self.live.get_mut(&terminal) else {
            return;
        };
        let Some(root) = &snapshot.root else {
            return;
        };
        let OpenEnum::Known(state) = root.report.state else {
            return;
        };
        live.last_status = Some(seq);
        let at = TurnPosition::new(root.turn, live.last_input);
        // A report the hub refuses (older than its last, or after exit) changes no fact
        // and so can satisfy no wait; nothing is lost by not applying it.
        match live
            .waits
            .report(StatusFact::new(root.report_seq, state, at))
        {
            Ok(()) | Err(_) => {}
        }
    }

    fn arming(&self, target: &Target) -> Result<(TerminalId, ArmedWait), BoundaryError> {
        let terminal = self.resolve_id(target)?;
        let armed = self.control.waits().arm(InterruptedWait::new(
            OpId::new(uuid::Uuid::now_v7().to_string()),
            WireTerminalId::new(terminal.to_string()),
            ByteOffset::new(0),
        ));
        Ok((terminal, armed))
    }

    /// Arms a turn wait bound to one send (`send_op`) or one turn number.
    pub(super) fn arm_turn_wait(
        &mut self,
        params: &TurnWaitParams,
    ) -> Result<Armed<TurnOutcome>, BoundaryError> {
        let (_, armed) = self.arming(&params.target)?;
        let live = live_terminal((&self.directory, &mut self.live), &params.target)?;
        let binding = match (&params.send_op, params.turn) {
            (Some(op), _) => live
                .sequencer
                .receipt(op)
                .map(TurnBinding::send)
                .ok_or_else(|| {
                    refusal("unknown_turn", "name the op_id of a send to this terminal")
                })?,
            (None, Some(turn)) => TurnBinding::Turn(turn),
            (None, None) => return Err(refusal("invalid_params", "name send_op or turn")),
        };
        let ticket = live
            .waits
            .turn(binding, armed.scope())
            .map_err(|e| wait_refusal(&e))?;
        Ok((ticket, deadline(params.timeout_ms)?, armed))
    }

    /// Arms a state wait; turn and cursor conditions have no counterpart in the hub.
    pub(super) fn arm_state_wait(
        &mut self,
        params: &EventsWaitParams,
    ) -> Result<(Armed<StateOutcome>, Option<EventSeq>), BoundaryError> {
        if params.turn_after.is_some() || params.cursor.is_some() {
            return Err(refusal(
                "unsupported_field",
                "wait on states or exit; turn_after and cursor are not served",
            ));
        }
        let (_, armed) = self.arming(&params.target)?;
        let live = live_terminal((&self.directory, &mut self.live), &params.target)?;
        let states: Vec<_> = params
            .states
            .iter()
            .flatten()
            .filter_map(|state| match state {
                OpenEnum::Known(state) => Some(*state),
                OpenEnum::Unknown(_) => None,
            })
            .collect();
        let wait =
            StateWait::new(&states, params.exit.unwrap_or(false)).map_err(|e| wait_refusal(&e))?;
        let ticket = live
            .waits
            .state(wait, armed.scope())
            .map_err(|e| wait_refusal(&e))?;
        Ok((
            (ticket, deadline(params.timeout_ms)?, armed),
            live.last_status,
        ))
    }

    /// Arms stop observation, then writes the interrupt key through the input path.
    pub(super) fn arm_interrupt(
        &mut self,
        session: &Session,
        params: &InputInterruptParams,
    ) -> Result<(Armed<Observed>, Receipt), BoundaryError> {
        let (_, armed) = self.arming(&params.target)?;
        let live = live_terminal((&self.directory, &mut self.live), &params.target)?;
        let ticket = live
            .waits
            .stop(StopWatch::new(), armed.scope())
            .map_err(|e| wait_refusal(&e))?;
        let key = [params
            .key
            .clone()
            .unwrap_or_else(|| INTERRUPT_KEY.to_owned())];
        let receipt = self.admit_receipt(
            session,
            Admission {
                target: &params.target,
                op_id: &params.op_id,
                lease: params.lease_id.as_ref(),
                payload: Payload::Keys(&key),
            },
        )?;
        Ok(((ticket, deadline(params.observe_ms)?, armed), receipt))
    }
}
