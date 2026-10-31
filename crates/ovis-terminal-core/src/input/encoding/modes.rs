//! Terminal modes frozen at one revision.

use ovis_terminal_vt::{ModeRevision, TerminalModes, Vt};

/// The structured modes of one terminal at one mode revision; every encoding is decided
/// against exactly one snapshot and carries its revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeSnapshot {
    revision: ModeRevision,
    modes: TerminalModes,
}

impl ModeSnapshot {
    /// Pairs modes with the revision they were read at.
    pub const fn new(revision: ModeRevision, modes: TerminalModes) -> Self {
        Self { revision, modes }
    }

    /// Reads the modes the program negotiated so far.
    pub const fn capture(vt: &Vt) -> Self {
        Self::new(vt.mode_revision(), vt.modes())
    }

    /// Revision the modes were read at.
    pub const fn revision(&self) -> ModeRevision {
        self.revision
    }

    /// The modes themselves.
    pub const fn modes(&self) -> &TerminalModes {
        &self.modes
    }
}
