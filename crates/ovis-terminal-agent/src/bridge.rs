//! The in-process prompt bridge for widgets without a terminal-key equivalent (plan todo 40).
//!
//! A program opens such a prompt through `prompt.open` with `mechanism: extension`; the
//! prompt service owns its versions, answers and receipts. The bridge only hands each
//! committed extension answer to the program's current session once, as a typed event,
//! and records the acknowledgement that names the version the answer committed.

mod deliver;
mod error;
mod session;

pub use error::BridgeError;
pub use session::{Attached, BridgeToken, PromptBridge};

#[cfg(test)]
mod qa;
