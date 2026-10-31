//! Unix holder service lifetime and endpoint discovery (plan todo 51).
//!
//! A session's shared holder runs as its own service process, owned by a systemd user
//! service, a launchd agent, or (explicitly, without logout survival) nobody. It
//! publishes a discovery record naming its socket and boot proof; observing that record
//! never creates, starts or registers anything, and stopping is an explicit operation
//! whose receipt enumerates what ended.

mod control;
mod definition;
mod error;
mod launch;
#[cfg(test)]
mod lifecycle_tests;
mod manager;
mod notify;
mod observe;
mod paths;
mod record;
mod registration;
mod run;
#[cfg(test)]
mod test_root;

pub use control::stop;
pub use error::ServiceError;
pub use launch::{launch_detached, ready_line};
pub use notify::notify_ready;
pub use observe::observe;
pub use paths::{ServiceEnvironment, ServicePaths, ServiceSession, ServiceVar};
pub use registration::{install, prerequisites, registered, settle, start, uninstall};
pub use run::run;
