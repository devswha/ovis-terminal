//! The one durable commit boundary shared by terminal actors, the registry and remote
//! mirrors (plan todo 42).
//!
//! Every source commits through the same [`Coordinator`](crate::events::Coordinator) and
//! its dedicated bounded store writer, so the sequence is allocated only at the
//! serialized durable commit and publication follows that commit. A remote mirror keeps
//! its upstream position as a [`RemoteOrigin`](crate::events::RemoteOrigin) beside the
//! local sequence; the boundary refuses a replayed or reordered upstream position in
//! the same transaction, so a mirrored record is never committed twice.
//!
//! [`Pager`] serves one pinned snapshot revision in bounded pages with an explicit
//! expiry; subscriptions install against the same coordinator barrier.

mod boundary;
mod error;
mod pager;
mod remote;
mod source;

pub use boundary::Boundary;
pub use error::CommitError;
pub use pager::{PageTtl, Pager, Window, WindowPage};
pub use source::{CommitSource, Committed};
