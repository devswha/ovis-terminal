//! One actor per machine: its own bounded queue, per-call deadlines, cancellation and
//! failure state, so a stalled or lost remote never holds local work or another machine.
//!
//! Callers never wait on a full queue: a call is refused as `backpressure` at once. A
//! call that outlives its deadline fails as `deadline_exceeded` and the peer is told to
//! cancel it. A lost line fails every call in flight as `machine_unreachable`; the machine
//! stays listed with its last-known state until a new line is attached. Reads of a
//! machine's health never message its actor.

mod actor;
mod error;
mod health;
mod line;
mod table;

pub use actor::{Inbound, Reply};
pub use error::ActorError;
pub use line::{Line, Lines};
pub use table::{Actors, Limits, Request};

#[cfg(test)]
mod qa;
