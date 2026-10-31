//! Typed errors crossing the JSON-RPC protocol boundary.

mod boundary;
mod candidate;
mod data;
mod deadline;
pub use candidate::TargetCandidate;
mod identity;
mod kinds;

pub use boundary::BoundaryError;
pub use data::ErrorData;
pub use identity::{FieldPath, OpId, TargetId};
pub use kinds::{ErrorKind, JsonRpcCode};
