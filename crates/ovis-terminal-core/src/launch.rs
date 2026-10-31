//! Readiness-bound first messages of launches (plan todo 35).
//!
//! A launch with a first message queues it in [`FirstMessages`] under the launch
//! operation and the requesting principal, and arms a declared-state wait on the new
//! terminal's [`crate::waits::Waits`] hub before the program is started, so a program
//! that declares readiness at once is never missed:
//!
//! - [`arm`] registers the wait; [`await_ready`] blocks until the program declares one
//!   of the template's ready states, the deadline passes, the program ends, or the
//!   runtime cancels the wait. Output silence is never readiness.
//! - [`FirstMessages::release`] runs once per message: the [`WriteBoundary`] rechecks
//!   the requesting principal's authority, the interact lease and the process
//!   incarnation at the moment of the write, encodes the text for the current modes and
//!   hands exactly one submission to the terminal's writer. Every other path ends the
//!   message `expired` or `cancelled` with zero bytes written.
//! - Only this first message waits for the declaration; ordinary input and stream
//!   geometry never do.

mod book;
mod boundary;
mod error;
mod readiness;
#[cfg(test)]
mod tests;

pub use book::{FirstMessages, QueuedMessage, Settlement};
pub use boundary::{TerminalState, WriteBoundary};
pub use error::LaunchError;
pub use readiness::{Verdict, arm, await_ready};
