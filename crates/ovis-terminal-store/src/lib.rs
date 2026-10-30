//! Atomic SQLite storage, operation receipts and quarantine of unsupported records.
//!
//! One dedicated writer thread commits each [`Mutation`] (compare-and-swap writes, its
//! events and its operation receipt) in a single WAL transaction with `synchronous=FULL`;
//! a commit is acknowledged only after `COMMIT` returns. Reads go through [`Snapshot`]s
//! pinned to one revision. Retention advances visible floors, deletes leave tombstones,
//! and a damaged or newer database is preserved with durable [`RecoveryEvidence`] instead
//! of being replaced by an empty store.

#![expect(
    clippy::redundant_pub_crate,
    reason = "rustc's unreachable_pub (workspace lint) requires pub(crate) on crate-internal items in private modules, which this nursery lint contradicts"
)]

mod apply;
mod error;
mod evidence;
mod ids;
mod mutation;
mod options;
mod quarantine;
mod schema;
mod snapshot;
mod store;
mod writer;

pub use error::StoreError;
pub use evidence::{
    MaterialDisposition, ProcessSurvival, RecoveryCause, RecoveryEvidence, RecoveryFlags,
    RelaunchPolicy,
};
pub use ids::{EventKind, OpId, Payload, ResourceKey, Revision, Version};
pub use mutation::{CasWrite, CommitReceipt, Mutation, NewEvent};
pub use options::{CommitBarrier, StoreLimits, StoreOptions};
pub use quarantine::read_quarantine_evidence;
pub use schema::SCHEMA_VERSION;
pub use snapshot::{EventPage, ReceiptLookup, ResourceLookup, Snapshot, StoredEvent};
pub use store::Store;
pub use writer::Ticket;

/// Cargo package name of this crate, used by the workspace layering record.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Workspace crates this crate builds on; every entry is a declared Cargo dependency.
pub const DEPENDS_ON: &[&str] = &[ovis_terminal_protocol::CRATE_NAME];

/// The bundled SQLite library version, recorded with evidence and diagnostics.
pub fn sqlite_version() -> &'static str {
    rusqlite::version()
}
