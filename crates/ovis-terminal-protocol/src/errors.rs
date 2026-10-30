//! Typed errors crossing the JSON-RPC protocol boundary.

mod boundary;
mod data;
mod identity;
mod kinds;

pub use boundary::BoundaryError;
pub use data::ErrorData;
pub use identity::{FieldPath, OpId, TargetId};
pub use kinds::{ErrorKind, JsonRpcCode};
