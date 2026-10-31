//! Ordered holder output fed into the VT engine in bounded slices, with the
//! raw ring, checkpoints and the query-reply lane advanced under one lock.

use std::sync::{Arc, Mutex, MutexGuard};

use ovis_terminal_holder::protocol::{Exit, Output, TerminalTarget};
use ovis_terminal_vt::{Checkpoint, ContentRevision, Geometry, OutputOffset, ScreenBuffer, Vt};

use super::config::{ScreenBounds, ScreenConfig};
use super::error::ScreenError;
use super::pins::Pins;
use super::replies::{ReplyBatch, ReplyLane};
use super::ring::OutputRing;
use super::views::{CheckpointMark, Ingested, LiveState, ScreenTransition, UnavailableReason};

#[derive(Debug)]
pub(super) struct Inner {
    pub(super) vt: Vt,
    pub(super) ring: OutputRing,
    pub(super) pins: Pins,
    pub(super) replies: ReplyLane,
    pub(super) checkpoint: Option<(CheckpointMark, Arc<Checkpoint>)>,
    since_checkpoint: usize,
    pub(super) ended: Option<OutputOffset>,
    checkpoint_interval: usize,
}

impl Inner {
    pub(super) const fn live(&self) -> LiveState {
        LiveState {
            revision: self.vt.revision(),
            offset: self.vt.offset(),
            at_live_bottom: true,
        }
    }

    const fn buffer(&self) -> ScreenBuffer {
        self.vt.modes().screen.buffer
    }

    /// Feeds one slice starting at `report.offset` and records what it changed.
    fn step(&mut self, slice: &[u8], report: &mut Ingested) -> Result<(), ScreenError> {
        if let Some(final_offset) = self.ended {
            return Err(ScreenError::Ended { final_offset });
        }
        if self.vt.offset() != report.offset {
            return Err(ScreenError::OutputGap {
                expected: self.vt.offset(),
                received: report.offset,
            });
        }
        let before = self.buffer();
        self.ring.append(slice);
        let fed = self.vt.feed(slice);
        for query in fed.queries {
            if self.replies.push(query) {
                report.queued_replies = report.queued_replies.saturating_add(1);
            }
        }
        report.osc.extend(fed.osc);
        report.offset = self.vt.offset();
        report.revision = self.vt.revision();
        let after = self.buffer();
        if before != after {
            report
                .transitions
                .push(if after == ScreenBuffer::Alternate {
                    ScreenTransition::HistoryUnavailable {
                        unavailability: UnavailableReason::AlternateScreen.into(),
                        offset: report.offset,
                        revision: report.revision,
                    }
                } else {
                    ScreenTransition::HistoryAvailable {
                        offset: report.offset,
                        revision: report.revision,
                    }
                });
        }
        self.since_checkpoint = self.since_checkpoint.saturating_add(slice.len());
        if self.since_checkpoint >= self.checkpoint_interval {
            report
                .transitions
                .push(ScreenTransition::Checkpoint(self.take_checkpoint()));
        }
        Ok(())
    }

    fn take_checkpoint(&mut self) -> CheckpointMark {
        let checkpoint = self.vt.export_checkpoint();
        let mark = CheckpointMark {
            offset: checkpoint.offset(),
            revision: checkpoint.revision(),
            fidelity: checkpoint.fidelity().clone(),
        };
        self.checkpoint = Some((mark.clone(), Arc::new(checkpoint)));
        self.since_checkpoint = 0;
        mark
    }
}

/// One terminal's authoritative screen, scrollback and raw output.
///
/// Clones share state: a producer ingests holder output while any number of
/// readers take revision-pinned immutable snapshots between work slices.
#[derive(Debug, Clone)]
pub struct Screen {
    target: Arc<TerminalTarget>,
    bounds: ScreenBounds,
    inner: Arc<Mutex<Inner>>,
}

impl Screen {
    /// Creates the screen of one terminal incarnation at offset zero.
    ///
    /// # Errors
    ///
    /// Returns [`ScreenError::InvalidConfig`] when retention cannot cover a checkpoint interval.
    pub fn new(target: TerminalTarget, config: ScreenConfig) -> Result<Self, ScreenError> {
        let bounds = config.bounds.validate()?;
        let inner = Inner {
            vt: Vt::new(config.vt),
            ring: OutputRing::new(bounds.retained_output()),
            pins: Pins::new(bounds.pinned_snapshots()),
            replies: ReplyLane::new(bounds.reply_capacity()),
            checkpoint: None,
            since_checkpoint: 0,
            ended: None,
            checkpoint_interval: bounds.checkpoint_interval(),
        };
        Ok(Self {
            target: Arc::new(target),
            bounds,
            inner: Arc::new(Mutex::new(inner)),
        })
    }

    /// The terminal incarnation this screen belongs to.
    pub fn target(&self) -> &TerminalTarget {
        &self.target
    }

    pub(super) fn lock(&self) -> Result<MutexGuard<'_, Inner>, ScreenError> {
        self.inner.lock().map_err(|_| ScreenError::Poisoned)
    }

    /// Ingests one holder output chunk in work-budget slices, releasing the
    /// lock between slices so reads never wait behind a large chunk.
    ///
    /// # Errors
    ///
    /// Returns target mismatch, output gap or ended rejections; bytes of a
    /// rejected slice are not applied.
    pub fn ingest(&self, output: &Output) -> Result<Ingested, ScreenError> {
        if output.target != *self.target {
            return Err(ScreenError::TargetMismatch);
        }
        let mut report = Ingested {
            offset: OutputOffset::new(output.offset),
            revision: ContentRevision::default(),
            transitions: Vec::new(),
            osc: Vec::new(),
            queued_replies: 0,
        };
        let empty: [&[u8]; 1] = [&[]];
        let slices: Vec<&[u8]> = if output.bytes.is_empty() {
            empty.to_vec()
        } else {
            output.bytes.chunks(self.bounds.work_budget()).collect()
        };
        for slice in slices {
            self.lock()?.step(slice, &mut report)?;
        }
        Ok(report)
    }

    /// Records the program's exit; history stays readable, or becomes
    /// permanently unavailable when the program ended full-screen.
    ///
    /// # Errors
    ///
    /// Returns target mismatch, or an output gap when output before the exit is missing.
    pub fn finish(&self, exit: &Exit) -> Result<Vec<ScreenTransition>, ScreenError> {
        if exit.target != *self.target {
            return Err(ScreenError::TargetMismatch);
        }
        let mut inner = self.lock()?;
        let final_offset = OutputOffset::new(exit.final_offset);
        if inner.vt.offset() != final_offset {
            return Err(ScreenError::OutputGap {
                expected: inner.vt.offset(),
                received: final_offset,
            });
        }
        if inner.ended.replace(final_offset).is_some() {
            return Ok(Vec::new());
        }
        let revision = inner.vt.revision();
        inner.pins.forget(revision);
        let buffer = inner.buffer();
        drop(inner);
        if buffer == ScreenBuffer::Primary {
            return Ok(Vec::new());
        }
        Ok(vec![ScreenTransition::HistoryUnavailable {
            unavailability: UnavailableReason::EndedInAlternateScreen.into(),
            offset: final_offset,
            revision,
        }])
    }

    /// Applies a grid change, reflowing the primary screen; pinned snapshots keep their grid.
    ///
    /// # Errors
    ///
    /// Returns [`ScreenError::Poisoned`] when the state lock is poisoned.
    pub fn resize(&self, geometry: Geometry) -> Result<ContentRevision, ScreenError> {
        let mut inner = self.lock()?;
        inner.vt.resize(geometry);
        Ok(inner.vt.revision())
    }

    /// The live revision and offset, without pinning a snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`ScreenError::Poisoned`] when the state lock is poisoned.
    pub fn live(&self) -> Result<LiveState, ScreenError> {
        Ok(self.lock()?.live())
    }

    /// The newest checkpoint, for recovery or a late attach.
    ///
    /// # Errors
    ///
    /// Returns [`ScreenError::Poisoned`] when the state lock is poisoned.
    pub fn checkpoint(&self) -> Result<Option<Arc<Checkpoint>>, ScreenError> {
        Ok(self
            .lock()?
            .checkpoint
            .as_ref()
            .map(|(_, checkpoint)| Arc::clone(checkpoint)))
    }

    /// Takes every queued terminal-query reply; each is returned exactly once.
    ///
    /// # Errors
    ///
    /// Returns [`ScreenError::Poisoned`] when the state lock is poisoned.
    pub fn drain_replies(&self) -> Result<ReplyBatch, ScreenError> {
        Ok(self.lock()?.replies.take())
    }
}
