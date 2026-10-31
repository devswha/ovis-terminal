//! Windows holder service host (plan todo 54).
//!
//! A session's holder runs as a Windows service the service control manager starts, so
//! it belongs to no client's or SSH session's job and outlives both. The host runs as
//! the account that owns the runtime, never a built-in service account, and serves the
//! holder wire on a named pipe whose DACL allows only that account. It publishes a
//! [`HostRecord`](ovis_terminal_protocol::domain::windows_service::HostRecord) naming the
//! pipe; observing that record never installs or starts anything, and every
//! administrative verb acts only on a service whose description marker it wrote.
//!
//! Naming, the service definition, the record and the observation rules build on every
//! host; the operating-system boundary (pipes, the service control manager, the holder
//! link and the service entry point) is Windows-only.

mod definition;
mod error;
mod observe;
mod paths;
mod record;

#[cfg(windows)]
mod facts;
#[cfg(windows)]
mod host;
#[cfg(test)]
#[cfg(windows)]
mod host_tests;
#[cfg(windows)]
mod link;
#[cfg(windows)]
pub mod pipe;
#[cfg(windows)]
mod scm;
#[cfg(windows)]
mod service_main;
#[cfg(windows)]
mod terminals;

pub use definition::{ServiceDefinition, marker, quote};
pub use error::HostError;
#[cfg(windows)]
pub use facts::{OsFacts, observe};
#[cfg(windows)]
pub use host::{HolderHost, HostConfig, Stopper};
#[cfg(windows)]
pub use link::LinkError;
pub use observe::{Facts, PipePresence, ProcessCheck, judge};
pub use paths::{HostPaths, HostSession};
pub use record::{read as read_record, withdraw as withdraw_record, write as write_record};
#[cfg(windows)]
pub use scm::{account_sid, install, installation, start, stop, uninstall};
#[cfg(windows)]
pub use service_main::run_service;
