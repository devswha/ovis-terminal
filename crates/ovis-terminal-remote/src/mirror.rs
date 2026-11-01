//! Remote events mirrored into the local sequence domain.
//!
//! Each remote event becomes one local commit whose local sequence is assigned by the
//! local coordinator in arrival order, while the remote machine, lifetime and sequence
//! travel beside it as [`RemoteOrigin`](ovis_terminal_core::events::RemoteOrigin) and the
//! record's source. Remote timestamps and clock skew are kept as metadata and never order
//! anything. A skipped remote sequence, a changed remote lifetime, a scan without events,
//! a lost line and a new line each become an explicit boundary record. A lost line marks
//! the machine's terminals unavailable at their last-known revision; only a remote
//! removal forgets one.

mod cursor;
mod error;
mod event;

pub use cursor::{Mirror, RemoteTerminalState};
pub use error::MirrorError;
pub use event::{MirrorEvent, Mirrored};

#[cfg(test)]
mod qa;
