//! Filesystem containment of caller-named paths below an authorized root (plan todo 58).
//!
//! The protocol's [`RelativePath`](ovis_terminal_protocol::domain::boundary_policy::RelativePath)
//! is already lexically contained; this module decides what the filesystem makes of it.
//! [`ScopedRoot::resolve`] follows symlinks and refuses a path that lands outside the
//! root, and [`ContainedPath::recheck`] repeats that decision at the moment of use, so a
//! symlink swapped in after authorization is refused instead of followed.

mod error;
mod identity;
mod root;

#[cfg(test)]
mod qa;

pub use error::PathError;
pub use root::{ContainedPath, ScopedRoot};
