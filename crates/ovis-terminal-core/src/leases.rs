//! Independent input and geometry lease ownership (plan todo 24).
//!
//! A terminal has one interact lease (input), one geometry lease (grid size) and any number
//! of observe leases; each lease has its own identity and each control slot its own epoch.
//! Rules:
//!
//! - Observers never change the input owner, the geometry owner or the grid.
//! - Acquiring a held slot returns `lease_conflict` naming the holder; nothing is evicted.
//! - Takeover needs the separate takeover grant plus the current epoch of the slot, and
//!   commits `lease.revoked` (naming the new owner) before the grant naming the old one.
//! - Release and disconnect revoke immediately. Geometry then passes to the interact-capable
//!   owner whose geometry request is oldest; activity and focus never reorder that queue.
//! - While any lease exists, writes need the current interact lease held by the same
//!   principal and client; knowing a lease id or a terminal address grants nothing.

mod admission;
mod error;
mod event;
mod lease;
mod requests;
mod table;
mod terminal;

pub use admission::LeaseView;
pub use error::LeaseError;
pub use event::{Granted, LeaseEvent, RevokeReason, Transition};
pub use lease::{ControlSlot, Lease, LeaseSlot, Owner};
pub use requests::{Acquire, Caller, Takeover, Write, WriteKind};
pub use terminal::{Authorization, TerminalLeases};
