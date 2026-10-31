//! Versioned documents committed by compare-and-swap (plan todo 50).
//!
//! The version check, the stored value, the `doc.changed` event and the operation receipt
//! are decided against one read snapshot and committed in one SQLite transaction. A rejected
//! write is retained as the operation's typed failure, so a retry with the same operation id
//! returns the same conflict instead of writing.

mod error;
mod event;
mod service;

#[cfg(test)]
mod qa;

pub use error::DocError;
pub use event::{DocChanged, DocumentsRetentionEvent};
pub use service::{DOC_MAX_BYTES, DocWrite, read, set_unit};
