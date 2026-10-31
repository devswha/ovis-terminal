//! `ovis-terminal service`: holder service lifetime and discovery on Unix (plan todo 51).

mod args;
mod execute;

pub use args::{ManagedArgs, Manager, Placement, ServiceCommand, SessionArgs, StartArgs};
pub use execute::execute;
