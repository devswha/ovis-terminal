//! Holder groups: the shared holder of one session and build, or the dedicated test
//! placement, each adopted over its own socket so a lost group affects only its terminals.

use std::path::PathBuf;

use ovis_terminal_holder::protocol::TerminalId;

/// Where a terminal's program is held.
///
/// [`Placement::Shared`] is the only production placement: one holder per session and
/// build serves every terminal. [`Placement::Dedicated`] is an internal test option that
/// gives one terminal its own holder process so holder-loss isolation can be exercised;
/// it is chosen by whoever starts holders and is never a field of the public v1 schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Placement {
    /// One holder per session and build.
    Shared,
    /// One holder for one terminal (internal test option).
    Dedicated,
}

/// The identity of one holder group.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroupId {
    placement: Placement,
    name: String,
}

impl GroupId {
    /// The shared holder of `session` and `build`.
    #[must_use]
    pub fn shared(session: &str, build: &str) -> Self {
        Self {
            placement: Placement::Shared,
            name: format!("{session}:{build}"),
        }
    }

    /// The dedicated holder of `terminal` (internal test option).
    #[must_use]
    pub fn dedicated(terminal: &TerminalId) -> Self {
        Self {
            placement: Placement::Dedicated,
            name: terminal.as_str().to_owned(),
        }
    }

    /// The group's placement.
    #[must_use]
    pub const fn placement(&self) -> Placement {
        self.placement
    }
}

/// A holder group and the socket it answers on.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct HolderGroup {
    /// Group identity.
    pub id: GroupId,
    /// Socket path.
    pub socket: PathBuf,
}

impl HolderGroup {
    /// `id` answering on `socket`.
    #[must_use]
    pub const fn new(id: GroupId, socket: PathBuf) -> Self {
        Self { id, socket }
    }
}
