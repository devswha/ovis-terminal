//! Windows pseudoconsole terminals (plan todo 53).
//!
//! A `ConptyTerminal` starts one program on a ConPTY through the pinned portable-pty
//! backend and owns it: a job object holds the program's process tree, a reader thread
//! always drains its output into a bounded [`model::OutputLog`], a writer thread
//! empties a bounded [`model::InputQueue`], and resize and kill act directly, never
//! behind input. Output bytes pass through untouched, including ConPTY's win32-input
//! request (`CSI ? 9001 h`); the core's VT engine owns modes and query replies.
//!
//! The holder that owns these terminals must itself be started by the service host
//! (plan todo 54), outside any SSH session's job; per-terminal jobs nest inside it.
//!
//! [`model`] holds the platform-neutral ownership rules and builds everywhere; the
//! operating-system boundary (`job`, `process`) is a pair of RAII wrappers whose
//! unsafe calls each carry their safety argument.

pub mod model;

#[cfg(windows)]
mod error;
#[cfg(windows)]
mod job;
#[cfg(windows)]
mod process;
#[cfg(windows)]
mod terminal;
#[cfg(test)]
#[cfg(windows)]
mod terminal_tests;
#[cfg(windows)]
mod workers;

#[cfg(windows)]
pub use error::ConptyError;
#[cfg(windows)]
pub use terminal::{ConptyTerminal, KILL_EXIT_CODE, TerminalLimits};
#[cfg(windows)]
pub use workers::InputProgress;
