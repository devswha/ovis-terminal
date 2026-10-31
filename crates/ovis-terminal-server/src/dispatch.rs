//! Typed method dispatch with deadlines and cancellation (plan todo 14).

mod deadline;
mod error;
mod execution;
mod job;
mod registry;
mod runtime;
mod tasks;

pub use error::DispatchError;
pub use execution::{Access, Completed, Execution};
pub use registry::Method;
pub use runtime::Dispatcher;
