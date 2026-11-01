//! Staged, verified updates with inspectable rollback outcomes (plan todo 47).
//!
//! A release archive is copied into the runtime install root only when it hashes to the
//! requested digest, verified (safe extraction, manifest, protocol and holder
//! compatibility) before it can be selected, and selected by one atomic file replacement
//! that stands only after the build proves ready and control restarted onto it. Every
//! step is journaled, so a failed or interrupted activation selects the previous build
//! again and leaves a rollback receipt. No program is ever signalled: holders keep their
//! programs and drain by build.

mod archive;
mod config;
mod control;
mod digest;
mod error;
mod journal;
mod layout;
mod probe;
mod service;
mod store;
#[cfg(all(test, unix))]
mod tests;
mod transition;
mod verify;

pub use config::{Config, CurrentBuild, Limits, Requirement};
pub use control::{RestartControl, RestartFailure};
pub use digest::Interrupt;
pub use error::UpdateError;
pub use service::Updates;
