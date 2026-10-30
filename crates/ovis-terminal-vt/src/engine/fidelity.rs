//! Whether engine state is an exact image of the output stream.

use serde::{Deserialize, Serialize};

/// Why a checkpoint or a restored engine cannot claim an exact image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InexactReason {
    /// The stream stopped inside a sequence after a control byte already acted,
    /// so replaying the open sequence would repeat that action.
    ControlInsideOpenSequence,
    /// A string longer than the parser bound was being discarded at the
    /// boundary; a client starting there sees its remaining bytes as text.
    OpenSequenceTooLong,
    /// The primary screen's saved cursor diverged from its cursor while the
    /// alternate screen was active; the emulator cannot rebuild that pair.
    PrimarySavedCursorUnrepresentable,
    /// A first paint had to spend the saved cursor (DECSC) to place a cursor
    /// outside the scroll region under origin mode.
    SavedCursorNotPaintable,
}

/// Exactness of a checkpoint or of the engine restored from it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "fidelity")]
#[non_exhaustive]
pub enum Fidelity {
    /// State plus retained bytes reproduce an always-live engine.
    #[default]
    Exact,
    /// State is the best reconstruction available, for the listed reasons.
    Inexact {
        /// Every reason that applies, without duplicates.
        reasons: Vec<InexactReason>,
    },
}

impl Fidelity {
    /// Whether the image is exact.
    pub const fn is_exact(&self) -> bool {
        matches!(self, Self::Exact)
    }

    pub(super) fn with(&self, reason: InexactReason) -> Self {
        let mut reasons = match self {
            Self::Exact => Vec::new(),
            Self::Inexact { reasons } => reasons.clone(),
        };
        if !reasons.contains(&reason) {
            reasons.push(reason);
        }
        Self::Inexact { reasons }
    }
}
