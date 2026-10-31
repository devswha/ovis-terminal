//! `ovis-terminal service`: the holder as a Windows service (plan todo 54).
//!
//! Installing, starting, stopping and uninstalling are explicit verbs that act only on a
//! service this runtime installed for its owner; `status` observes and never starts.

mod args;
mod execute;
#[cfg(test)]
mod fixture_tests;

pub use args::{HostArgs, InstallArgs, Placement, RunArgs, ServiceCommand};
pub use execute::execute;
