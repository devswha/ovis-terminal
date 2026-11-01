//! Unix PTY holders and bounded pumps (plan todo 6).
//!
//! One holder serves every terminal of a session and build (the shared holder):
//! it is the parent of each program, always drains its PTY into a bounded raw
//! tail, queues input per terminal within byte and entry bounds, and applies resize and
//! signals without waiting behind that input. A core adopts it over a Unix
//! socket; when the core disconnects or crashes, programs keep running and the
//! next adoption sees the same PID and start token.
//!
//! Dedicated isolation is an internal placement of this same holder with a
//! single terminal, chosen by whoever launches holders; it is not a field of
//! the v1 public schema. Holders parse no VT, persist no state, infer no
//! status and apply no policy.

mod config;
mod core_connection;
mod detach;
mod drain;
mod error;
mod holder;
#[cfg(test)]
mod holder_tests;
mod input;
#[cfg(test)]
mod input_tests;
mod io_error;
mod join_report;
mod launch;
mod launch_error;
#[cfg(test)]
mod launch_tests;
mod link;
mod pty_io;
mod service;
mod service_error;
#[cfg(test)]
mod service_tests;
mod tail;
#[cfg(test)]
mod test_fixtures;

pub use config::{HolderConfig, HolderLimits, ProcessEnvironment, QueueLimits};
pub use core_connection::{CoreConnection, CoreConnectionError};
pub use detach::spawn_detached;
pub use error::HolderError;
pub use holder::Holder;
pub use io_error::TerminalIoError;
pub use join_report::{JoinOutcome, ShutdownReport, WorkerRole, WorkersJoined};
pub use launch::{Grid, LaunchRequest, LaunchSpec};
pub use launch_error::{LaunchError, PtySourceError};
pub use link::{CoreLink, LinkClosed, RecvError};
pub use service::HolderService;
pub use service_error::{ConnectionError, ServeError};
