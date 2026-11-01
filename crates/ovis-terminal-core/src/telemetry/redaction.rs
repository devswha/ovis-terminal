//! The one redaction boundary for diagnostics, audit records and stored receipts (plan todo 58).
//!
//! Every structured record leaving the runtime for a log, an audit trail or an error reply
//! passes [`redact`]: credential-bearing and secret-input fields are replaced wherever they
//! sit, and bearer-shaped text is scrubbed from every other string. [`RedactingSink`] applies
//! it to a [`TelemetrySink`](super::TelemetrySink); [`AuditLog`] applies it before a record
//! is retained and enforces the retention policy; [`seal`] keeps only a digest of a secret
//! that must persist in the operation ledger.

mod retention;
mod rules;
mod sink;

#[cfg(test)]
mod qa;

pub use retention::{AuditLog, RetentionPolicy};
pub use rules::{REDACTED, redact, redacted, scrub, seal};
pub use sink::RedactingSink;
