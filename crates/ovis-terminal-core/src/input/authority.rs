//! What a credential change does to admitted input (plan todo 57).
//!
//! Admission decides a write against the writer's credential, lease and grant, and the
//! sequencer's write boundary decides it again. When a credential is rotated or revoked the
//! runtime calls [`recheck`] on each terminal's sequencer at once instead of waiting for the
//! next write, so the caller learns, per write, whether it was cancelled before commit,
//! already committed with the holder's delivery facts, or of unknown outcome because it is
//! with the holder and the fence has not been answered. Nothing is ever resent.

mod race;

pub use race::{Recheck, race, raced, recheck};
