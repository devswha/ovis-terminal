//! Planned control restart that keeps every program incarnation (plan todo 48).
//!
//! [`Control`] owns the terminal lifecycle actor and replaces it through an explicit
//! state machine ([`RestartPhase`]): the intent and the preserved incarnations are made
//! durable first, readiness is withdrawn, the boundary is announced and armed waits are
//! interrupted, holder ownership is handed over without signalling a program, and the
//! successor adopts the holder under a new server boot identity read from the intent.
//! Only then are the preserved incarnations verified by PID and start token and the
//! interrupted operations classified from the committed ledger and holder write
//! evidence. A crash at any barrier leaves the intent on disk; the next
//! [`Control::open`] finishes the same recovery instead of guessing.
//!
//! Stop, stop-programs, kill and metadata removal stay separate verbs with their own
//! receipts; nothing here signals a process except [`Control::stop`] with
//! `control_and_programs`.

mod classify;
#[cfg(unix)]
mod control;
mod error;
mod intent;
#[cfg(all(test, unix))]
mod qa;
mod receipt;
#[cfg(unix)]
mod sequence;
mod waits;

pub use classify::classify;
#[cfg(unix)]
pub use control::{Control, ControlPlan};
pub use error::RestartError;
pub use intent::{IntentFile, RestartIntent};
pub use ovis_terminal_protocol::domain::restart::RestartPhase;
pub use receipt::{Preserved, RestartReceipt, StopReceipt};
pub use waits::{ArmedWait, ArmedWaits};
