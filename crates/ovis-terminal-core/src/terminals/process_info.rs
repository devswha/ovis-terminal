//! Process incarnations reported the same way on every operating system (plan todo 55).
//!
//! Linux proves a process by `/proc` start ticks, macOS by its `proc_pidinfo` start
//! time and Windows by its creation time. Roles come from one bounded process-table
//! pass; a field the OS cannot prove is an explicit unsupported value, and only a
//! changed or vanished start token proves that an owned process ended.

mod liveness;
mod roles;
mod table;
mod wire;

pub use liveness::{Liveness, liveness};
pub use roles::{observe, unavailable};
pub use table::{NativeProcess, NativeTable, ProcessTable, Session, Terminal};

#[cfg(test)]
mod tests;
