//! Bounded VT scheduling with control-first terminal actors (plan todo 56).
//!
//! Each terminal has one actor on the multi-thread runtime. Control requests (cancel,
//! resize, stop) have their own reserved queue and are served before admissions, output
//! intake and parse turns; a parse turn feeds at most one [`ParseBudget`] and yields.
//! Input is reserved against per-terminal and session byte budgets before any buffer is
//! allocated and is written by a separate task, so a program that does not read stdin
//! stays controllable. Program output arrives from owned pump threads through a bounded
//! feed and fans out as one shared chunk per event. Synchronous database work runs on a
//! dedicated bounded writer thread or through a bounded blocking gate.
//!
//! [`ParseBudget`]: ovis_terminal_vt::ParseBudget

mod actor;
mod blocking;
mod budget;
mod config;
mod db;
mod error;
mod event;
mod handle;
mod input;
mod pump;
mod writer;

#[cfg(test)]
mod qa;

pub use actor::{ActorReport, ActorTask, spawn_terminal};
pub use blocking::BlockingGate;
pub use budget::{ByteBudget, Reservation};
pub use config::{ActorConfig, TerminalSetup};
pub use db::DbWriter;
pub use error::{BudgetScope, SchedulerError};
pub use event::{OutputEvent, TerminalEvent};
pub use handle::{OutputFeed, Receipt, TerminalHandle};
pub use input::{CancelOutcome, Reserved, Ticket};
pub use pump::{PumpEnd, PumpReport, PumpThread};
pub use writer::WriterReport;
