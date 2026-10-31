//! Process incarnation evidence gathered by the holder (plan todo 6).
//!
//! A holder reports each child by PID plus an operating-system start token, so
//! a reused PID is never mistaken for the process the holder started.

#[cfg(unix)]
mod signal;
#[cfg(unix)]
mod start_token;

#[cfg(unix)]
pub use signal::{SignalError, SignalScope, send_hangup, send_signal};
#[cfg(unix)]
pub use start_token::{StartTokenError, read_start_token};

pub use crate::protocol::ProcessId;
