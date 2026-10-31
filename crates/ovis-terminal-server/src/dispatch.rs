//! Typed method dispatch with deadlines and cancellation (plan todo 14).

mod control;
mod deadline;
mod error;
mod execution;
mod job;
/// Domain lanes registered on the dispatcher by assembly.
pub mod lanes {
    pub mod worktrees;
}
mod registry;
mod runtime;
mod tasks;

pub use error::DispatchError;
pub use execution::{Access, Completed, Execution};
pub use registry::Method;
pub use runtime::Dispatcher;
