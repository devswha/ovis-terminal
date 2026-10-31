//! Session binding and versioned conversation history cursors (plan todo 36).
//!
//! The owning process of a registered agent declares every session change explicitly:
//! the first binding, a resume, a branch selection or a replacement. Each history
//! identity keeps the source it was first bound to; its revision advances whenever an
//! observation finds the transcript appended, truncated, replaced or rewritten in place,
//! or when the owning process selects another chain. Cursors carry identity and revision,
//! so a read against a moved history is refused instead of mixing records.

mod chain;
mod error;
mod observe;
mod page;
mod request;
mod row;
mod service;
mod source;

pub use error::BindingError;
pub use request::{Read, Update};
pub use service::BindingService;
pub use source::{SessionKey, SourceRef, Stores, TranscriptPath};

#[cfg(test)]
mod qa;
