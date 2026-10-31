//! Typed method dispatch with deadlines and cancellation (plan todo 14).

mod control;
mod deadline;
mod error;
mod execution;
mod job;
mod read;
pub mod lanes;
mod registry;
mod runtime;
mod tasks;

pub use error::DispatchError;
pub use execution::{Access, Completed, Execution};
pub use read::Read;
pub use registry::Method;
pub use runtime::Dispatcher;
