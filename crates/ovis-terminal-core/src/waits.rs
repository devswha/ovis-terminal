//! Atomic cursor waits and evidenced interrupt outcomes (plan todo 27).
//!
//! One [`Waits`] hub per terminal incarnation owns the path holder output, exit and
//! declared status take into the core, and the table of armed observers:
//!
//! - Every registration evaluates the current facts and, when they do not already
//!   satisfy it, inserts its observer in the same critical section that every feed
//!   takes, so no output byte, exit or status report can fall between the check and
//!   the subscription. An already satisfied wait answers at once.
//! - An output wait starts at a cursor captured before the send (the admission
//!   receipt's `output_cursor`, or [`Waits::cursor`]) and matches retained output since
//!   that boundary, so a marker printed while the send was in flight is found. A cursor
//!   behind retention is rejected with a typed `output_gap`, never a timeout.
//! - A turn wait is bound to one send (its input sequence) or one turn number; a turn
//!   that started before the send reached the program never satisfies it, and the
//!   bound passing is `timeout`, never completion.
//! - An interrupt keeps the delivered action apart from the observed outcome and claims
//!   `stopped` only with evidence: the exit of this process incarnation, or a declared
//!   status report that left the working state after the interrupt was armed. Screen
//!   text and a successful key write are never evidence.
//! - Waits end at their deadline or when their [`CancelScope`] is cancelled; either way
//!   the observer leaves the table at once. Waiting blocks on a condition variable
//!   that a feed signals; nothing polls.

mod board;
mod cancel;
mod entry;
mod error;
mod facts;
mod hub;
mod interrupt;
mod output;
mod scan;
mod slot;
mod state;
mod turn;

pub use cancel::CancelScope;
pub use error::WaitError;
pub use facts::{ExitEvidence, ProcessState, StatusFact, TurnPosition};
pub use hub::Waits;
pub use interrupt::{
    InterruptReport, Observed, ProcessObservation, StopEvidence, StopWatch, WatchEnd,
};
pub use output::{OutputOutcome, OutputWait};
pub use slot::Ticket;
pub use state::{Satisfied, StateOutcome, StateWait};
pub use turn::{TurnBinding, TurnOutcome};
