//! Staged input delivery receipts (plan todo 25).
//!
//! A receipt is created at admission and answers immediately with the entry's position in
//! the terminal's input sequence; every later stage is filled only by evidence:
//!
//! - `delivery` stays `queued` until the holder acknowledges the actual PTY write, then
//!   reports the exact byte count (`pty_written` or `partial`), a typed `fenced` reason when
//!   no byte was written, or `outcome_unknown` when evidence ended before an acknowledgement.
//!   Nothing ever claims a written byte was taken back.
//! - `agent_accepted`, `turn_started` and `turn_ended` are `unknown` unless a declared producer
//!   can report them, then `pending` until it does; PTY delivery never fills them.

mod book;
mod error;
mod receipt;
mod seq;

pub(in crate::input) use book::ReceiptBook;
pub use error::ReceiptError;
pub use receipt::{AgentStage, Delivery, FenceReason, Receipt, Reporter, Stage, StageReport};
pub use seq::InputSeq;
