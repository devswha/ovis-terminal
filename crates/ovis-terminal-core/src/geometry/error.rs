//! The geometry boundary error, its stable kinds and recovery hints.

use ovis_terminal_vt::VtError;

use super::GeometryEpoch;
use crate::leases::LeaseError;
use crate::screen::ScreenError;

/// Why a geometry operation was refused; the grid and readiness are unchanged.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GeometryError {
    /// The lease boundary refused the caller.
    #[error("the lease boundary refused this geometry operation")]
    Lease(#[source] LeaseError),
    /// The requested size is not a terminal grid.
    #[error("{cols}x{rows} is not a terminal grid")]
    InvalidGeometry {
        /// Requested columns.
        cols: u16,
        /// Requested rows.
        rows: u16,
        /// The emulator's refusal.
        #[source]
        source: VtError,
    },
    /// Evidence named an epoch that is no longer being applied.
    #[error("epoch {} is not the transition in progress; epoch {} is current", observed.get(), current.get())]
    StaleGeometry {
        /// Epoch the evidence named.
        observed: GeometryEpoch,
        /// Newest epoch issued.
        current: GeometryEpoch,
    },
    /// Evidence named an epoch that was never issued.
    #[error("epoch {} was never issued; epoch {} is the newest", observed.get(), current.get())]
    UnknownEpoch {
        /// Epoch the evidence named.
        observed: GeometryEpoch,
        /// Newest epoch issued.
        current: GeometryEpoch,
    },
    /// A grid change is still being applied; input and repaint wait for its completion.
    #[error("geometry epoch {} is still being applied", pending.get())]
    NotReady {
        /// Epoch whose completion re-asserts input readiness.
        pending: GeometryEpoch,
    },
    /// The screen pipeline refused.
    #[error("the screen refused the geometry operation")]
    Screen(#[source] ScreenError),
    /// The geometry epoch cannot advance.
    #[error("the geometry epoch cannot advance")]
    Exhausted,
}

impl GeometryError {
    /// The stable machine-readable kind clients branch on.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Lease(source) => source.kind(),
            Self::InvalidGeometry { .. } => "invalid_geometry",
            Self::StaleGeometry { .. } => "stale_geometry",
            Self::UnknownEpoch { .. } => "invalid_params",
            Self::NotReady { .. } => "not_ready",
            Self::Screen(source) => source.kind(),
            Self::Exhausted => "limit_exceeded",
        }
    }

    /// Recovery guidance published with the kind.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Lease(source) => source.hint(),
            Self::InvalidGeometry { .. } => "request at least two columns and one row",
            Self::StaleGeometry { .. } => "act on the newest geometry.changed epoch",
            Self::UnknownEpoch { .. } => "name an epoch from a geometry.changed event",
            Self::NotReady { .. } => "wait for geometry.input_ready at the pending epoch",
            Self::Screen(source) => source.hint(),
            Self::Exhausted => "recover the terminal to start a new incarnation",
        }
    }
}
