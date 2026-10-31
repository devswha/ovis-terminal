//! Operation deduplication and inspectable outcomes (plan todo 15).

mod completion;
mod context;
mod error;
mod evidence;
mod ledger;
mod request;
mod transitions;

pub use completion::Completion;
pub use context::{Context, Operations, RunningOperation};
pub use error::OperationError;
pub use ledger::{OperationEvent, OperationProducer, OperationSnapshot};
pub use request::Request;

/// Advertised deduplication retention: 24 hours after acceptance.
pub const OP_RETENTION_MS: ovis_terminal_protocol::schema::scalar::DurationMs =
    ovis_terminal_protocol::schema::scalar::DurationMs::new(86_400_000);
