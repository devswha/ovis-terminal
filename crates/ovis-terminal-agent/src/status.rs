//! Declared agent registration and authoritative status (plan todo 33).
//!
//! Only two writers change lifecycle status: an OSC 7501 record parsed from the
//! program's own output stream, and a `status.report` by the process that owns the
//! terminal's current program incarnation. Screen text, process names and provider
//! stores have no path into this module.

mod error;
mod event;
mod ingest;
mod machine;
mod osc;
mod owner;
mod request;
pub mod row;
mod service;

pub use error::StatusError;
pub use event::AgentStatusEvent;
pub use ingest::{Ingested, OscOutput};
pub use osc::{OscOutcome, OscReport, classify};
pub use owner::{ProcessOwner, ProcessTokens};
pub use request::{Clear, Register, Report};
pub use service::StatusService;

#[cfg(test)]
mod qa;
