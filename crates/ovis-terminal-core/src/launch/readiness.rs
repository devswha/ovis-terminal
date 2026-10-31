//! The declared-state wait that gates a first message.

use std::time::Instant;

use ovis_terminal_protocol::domain::template_launch::FirstMessageReason;
use ovis_terminal_protocol::schema::records::AgentState;

use super::{LaunchError, Settlement};
use crate::waits::{CancelScope, StateOutcome, StateWait, Ticket, Waits};

/// What a readiness wait decided for its message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Verdict {
    /// The program declared a ready state; write the message.
    Ready,
    /// Never write the message.
    Settle(Settlement),
}

/// Arms the wait for a declared ready state; arm it before the program starts.
///
/// A declaration already fed to `waits` satisfies it at once.
///
/// # Errors
///
/// `invalid_params` when `states` is empty, or the hub's failure.
pub fn arm(
    waits: &Waits,
    states: &[AgentState],
    scope: &CancelScope,
) -> Result<Ticket<StateOutcome>, LaunchError> {
    let wait = StateWait::new(states, false).map_err(LaunchError::Wait)?;
    waits.state(wait, scope).map_err(LaunchError::Wait)
}

/// Blocks until readiness is declared, the program ends, the scope is cancelled or
/// `deadline` passes.
///
/// # Errors
///
/// The hub's failure.
pub fn await_ready(
    ticket: Ticket<StateOutcome>,
    deadline: Instant,
) -> Result<Verdict, LaunchError> {
    let outcome = ticket.wait(deadline).map_err(LaunchError::Wait)?;
    Ok(match outcome {
        StateOutcome::AlreadySatisfied { .. } | StateOutcome::Matched { .. } => Verdict::Ready,
        StateOutcome::Timeout => Verdict::Settle(Settlement::Expired),
        StateOutcome::Exited { .. } => {
            Verdict::Settle(Settlement::Cancelled(FirstMessageReason::ProcessExited))
        }
        StateOutcome::Lost => Verdict::Settle(Settlement::Cancelled(FirstMessageReason::LinkLost)),
        StateOutcome::Cancelled => {
            Verdict::Settle(Settlement::Cancelled(FirstMessageReason::RuntimeStopped))
        }
    })
}
