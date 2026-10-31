//! Per-terminal screen bounds: raw retention, VT work slices, checkpoint
//! spacing, pinned snapshots and the query-reply lane.

use std::num::NonZeroUsize;

use ovis_terminal_vt::VtConfig;

use super::error::ScreenError;

const fn bound(value: usize) -> NonZeroUsize {
    match NonZeroUsize::new(value) {
        Some(value) => value,
        None => NonZeroUsize::MIN,
    }
}

/// Admission bounds of one terminal's screen pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenBounds {
    retained_output: NonZeroUsize,
    work_budget: NonZeroUsize,
    checkpoint_interval: NonZeroUsize,
    pinned_snapshots: NonZeroUsize,
    reply_capacity: NonZeroUsize,
}

impl ScreenBounds {
    /// 8 MiB raw retention, 64 KiB VT slices, a checkpoint per MiB, eight
    /// pinned snapshots and 64 queued query replies.
    pub const DEFAULT: Self = Self {
        retained_output: bound(8 * 1024 * 1024),
        work_budget: bound(64 * 1024),
        checkpoint_interval: bound(1024 * 1024),
        pinned_snapshots: bound(8),
        reply_capacity: bound(64),
    };

    /// Raw output bytes kept for `output` reads; older bytes advance the floor.
    #[must_use]
    pub const fn with_retained_output(self, bytes: NonZeroUsize) -> Self {
        Self {
            retained_output: bytes,
            ..self
        }
    }

    /// Bytes parsed per locked step, so readers interleave with a busy producer.
    #[must_use]
    pub const fn with_work_budget(self, bytes: NonZeroUsize) -> Self {
        Self {
            work_budget: bytes,
            ..self
        }
    }

    /// Output bytes between two VT checkpoints.
    #[must_use]
    pub const fn with_checkpoint_interval(self, bytes: NonZeroUsize) -> Self {
        Self {
            checkpoint_interval: bytes,
            ..self
        }
    }

    /// Snapshots kept for revision-pinned reads.
    #[must_use]
    pub const fn with_pinned_snapshots(self, count: NonZeroUsize) -> Self {
        Self {
            pinned_snapshots: count,
            ..self
        }
    }

    /// Query replies queued before further replies are counted as dropped.
    #[must_use]
    pub const fn with_reply_capacity(self, count: NonZeroUsize) -> Self {
        Self {
            reply_capacity: count,
            ..self
        }
    }

    pub(super) const fn retained_output(self) -> usize {
        self.retained_output.get()
    }

    pub(super) const fn work_budget(self) -> usize {
        self.work_budget.get()
    }

    pub(super) const fn checkpoint_interval(self) -> usize {
        self.checkpoint_interval.get()
    }

    pub(super) const fn pinned_snapshots(self) -> usize {
        self.pinned_snapshots.get()
    }

    pub(super) const fn reply_capacity(self) -> usize {
        self.reply_capacity.get()
    }

    /// A checkpoint lies at most `interval + budget - 1` bytes behind the head,
    /// so retention must cover that distance for the newest checkpoint plus
    /// retained bytes to reconstruct the screen.
    pub(super) fn validate(self) -> Result<Self, ScreenError> {
        let reach = self
            .checkpoint_interval()
            .checked_add(self.work_budget())
            .ok_or(ScreenError::InvalidConfig {
                reason: "checkpoint interval plus work budget overflows",
            })?;
        if reach > self.retained_output() {
            return Err(ScreenError::InvalidConfig {
                reason: "retained output must cover the checkpoint interval plus one work slice",
            });
        }
        Ok(self)
    }
}

/// Emulator configuration plus pipeline bounds for one terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenConfig {
    pub(super) vt: VtConfig,
    pub(super) bounds: ScreenBounds,
}

impl ScreenConfig {
    /// Builds a configuration.
    pub const fn new(vt: VtConfig, bounds: ScreenBounds) -> Self {
        Self { vt, bounds }
    }
}
