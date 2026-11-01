//! One screen per recorded program: the core-owned engine fed with contiguous holder
//! output, and its offset-fenced checkpoints.

use ovis_terminal_core::recovery::{CheckpointStore, Restored};
use ovis_terminal_holder::protocol::TerminalTarget;
use ovis_terminal_protocol::schema::scalar::ByteOffset;
use ovis_terminal_vt::{Vt, VtConfig};

use super::super::error::SoakError;

#[derive(Debug)]
pub(super) struct Screen {
    pub(super) target: TerminalTarget,
    vt: Vt,
    /// Stream offset the next holder byte must have.
    next: u64,
    /// Bytes the engine never saw: its own offset trails the stream by this much.
    lag: u64,
    saved: Option<u64>,
    /// Fixture: keep the first checkpoint and never refresh it.
    frozen: bool,
}

impl Screen {
    pub(super) fn fresh(target: TerminalTarget, config: VtConfig) -> Self {
        Self {
            target,
            vt: Vt::new(config),
            next: 0,
            lag: 0,
            saved: None,
            frozen: false,
        }
    }

    pub(super) fn restored(target: TerminalTarget, restored: Restored) -> Self {
        let checkpoint = restored.screen.checkpoint_offset.map(ByteOffset::get);
        Self {
            target,
            next: restored.screen.head.get(),
            lag: restored.lag,
            saved: checkpoint,
            vt: restored.vt,
            frozen: false,
        }
    }

    pub(super) const fn freeze(&mut self) {
        self.frozen = true;
    }

    pub(super) const fn unfreeze(&mut self) {
        self.frozen = false;
    }

    pub(super) const fn head(&self) -> u64 {
        self.next
    }

    pub(super) const fn lag(&self) -> u64 {
        self.lag
    }

    /// Feeds the part of `bytes` (starting at stream offset `offset`) not seen yet. A
    /// range that starts past the head is a holder skip: it is counted as lag, never filled.
    pub(super) fn admit(&mut self, offset: u64, bytes: &[u8]) {
        if offset > self.next {
            self.lag = self.lag.saturating_add(offset.saturating_sub(self.next));
            self.next = offset;
        }
        let skip = usize::try_from(self.next.saturating_sub(offset)).unwrap_or(usize::MAX);
        let fresh = bytes.get(skip..).unwrap_or_default();
        if fresh.is_empty() {
            return;
        }
        self.vt.feed(fresh);
        self.next = self
            .next
            .saturating_add(u64::try_from(fresh.len()).unwrap_or(u64::MAX));
    }

    /// Saves a checkpoint at the head once there is output, when it moved, the engine is
    /// whole and the fixture does not hold the old one; returns the saved offset.
    pub(super) fn checkpoint(&mut self, store: &CheckpointStore) -> Result<Option<u64>, SoakError> {
        let held = self.frozen && self.saved.is_some();
        if held || self.next == 0 || self.lag > 0 || self.saved == Some(self.next) {
            return Ok(None);
        }
        store.save(&self.target, &self.vt.export_checkpoint())?;
        self.saved = Some(self.next);
        Ok(self.saved)
    }
}
