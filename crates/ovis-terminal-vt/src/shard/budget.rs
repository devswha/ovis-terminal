//! How much one parse turn may consume and how much output may wait for parsing.

use std::num::NonZeroUsize;

const KIB: usize = 1024;

/// Bytes one parse turn may feed to the engine before its owner yields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseBudget(NonZeroUsize);

impl ParseBudget {
    /// The measured default: 16 KiB per turn.
    ///
    /// `bench/tuning` measured `alacritty_terminal` at 27 to 70 ns per byte in release
    /// builds (28 to 68 ms per MiB); 16 KiB keeps one turn under 2 ms, so a control
    /// request waits at most a few turns of the 100 ms budget.
    pub const DEFAULT: Self = match NonZeroUsize::new(16 * KIB) {
        Some(bytes) => Self(bytes),
        None => Self(NonZeroUsize::MIN),
    };

    /// A budget of `bytes` per turn.
    #[must_use]
    pub const fn new(bytes: NonZeroUsize) -> Self {
        Self(bytes)
    }

    /// Bytes per turn.
    #[must_use]
    pub const fn bytes(self) -> usize {
        self.0.get()
    }
}

/// Output bytes a shard holds unparsed before it refuses more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingCap(usize);

impl PendingCap {
    /// The default: 1 MiB, so 50 terminals stay inside a 64 MiB session backlog.
    pub const DEFAULT: Self = Self(KIB * KIB);

    /// A cap of `bytes`.
    #[must_use]
    pub const fn new(bytes: usize) -> Self {
        Self(bytes)
    }

    /// Bytes allowed to wait.
    #[must_use]
    pub const fn bytes(self) -> usize {
        self.0
    }
}
