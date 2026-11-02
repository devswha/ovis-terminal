//! Retained graphics re-sent after a reattach or a resync (plan todo 84).
//!
//! Every graphics unit the gate releases is offered to the terminal's image store,
//! which keeps it while the VT engine still shows its placement and evicts the oldest
//! past `graphics_terminal_store_max_bytes`; an eviction is reported in-stream by one
//! `graphics_evicted` record after the sequence that caused it. A checkpoint paint names
//! where each image still shown goes, after its screen's rows; the paint is sent as
//! pieces that share the store's bytes, byte-identical and in receipt order, followed by
//! one `graphics_replayed` record naming what was re-sent and what was released.

#[cfg(test)]
mod tests;

use std::sync::Arc;

use ovis_terminal_core::terminals::image_placeholders;
use ovis_terminal_core::terminals::images::ImageStore;
use ovis_terminal_protocol::domain::graphics::{GraphicsPlaceholder, GraphicsReplayedData};
use ovis_terminal_protocol::schema::ids::StreamId;
use ovis_terminal_protocol::schema::records::{AttachControl, ControlKind};
use ovis_terminal_protocol::schema::scalar::ByteOffset;
use ovis_terminal_vt::{FirstPaint, ImageSet, OutputOffset, ScreenImage};

use super::super::error::StreamError;
use super::super::hub::{HubState, TerminalStreams};
use super::super::log::{NoticeData, Shared};
use super::super::viewer::Viewer;
use super::Gated;

/// One piece of a paint; `replay` marks retained graphics bytes, which the image store
/// bounds rather than the stream backlog.
#[derive(Debug, Clone)]
pub(in crate::streams) struct Piece {
    pub bytes: Shared,
    pub replay: bool,
}

/// A fenced paint, with the retained graphics it re-sends, shared by every viewer
/// resynchronized at the same boundary.
#[derive(Debug)]
pub(in crate::streams) struct Painted {
    pub paint: FirstPaint,
    pub pieces: Vec<Piece>,
    /// The `graphics_replayed` data, when anything was re-sent or released.
    pub replay: Option<GraphicsReplayedData>,
}

impl Painted {
    /// Queues the `graphics_replayed` record after `viewer`'s paint, if there is one.
    pub(in crate::streams) fn announce(&self, viewer: &mut Viewer) {
        if let Some(data) = &self.replay {
            let at = viewer.next_offset().wire();
            let mut record =
                AttachControl::new(viewer.id.clone(), ControlKind::GraphicsReplayed, at);
            record.graphics_replayed = Some(data.clone());
            viewer.control(record);
        }
    }
}

/// Appends `gated` to the log once its released units were offered to the store: each
/// eviction stands after the sequence that caused it, a series dropped over the
/// sequence cap leaves the store, and images `shown` no longer retains are forgotten.
pub(in crate::streams) fn admit(state: &mut HubState, mut gated: Gated, shown: &ImageSet) {
    for (index, unit) in std::mem::take(&mut gated.units) {
        if let Some(notice) = state.images.admit(unit, shown) {
            gated.evicted.push((index, notice));
        }
    }
    for (_, dropped) in &gated.dropped {
        state
            .images
            .forget(OutputOffset::new(dropped.output_offset.get()));
    }
    state.images.sync(shown);
    state.log.append(gated);
}

/// The record a graphics notice is sent as on stream `id` at stream offset `at`.
pub(in crate::streams) fn notice_record(
    id: &StreamId,
    at: ByteOffset,
    data: &NoticeData,
) -> AttachControl {
    match data {
        NoticeData::Dropped(dropped) => {
            let mut record = AttachControl::new(id.clone(), ControlKind::GraphicsDropped, at);
            record.graphics_dropped = Some(dropped.clone());
            record
        }
        NoticeData::Evicted(evicted) => {
            let mut record = AttachControl::new(id.clone(), ControlKind::GraphicsEvicted, at);
            record.graphics_evicted = Some(evicted.clone());
            record
        }
    }
}

fn plain(bytes: &Arc<[u8]>, range: std::ops::Range<usize>) -> Option<Piece> {
    (range.end > range.start).then(|| Piece {
        bytes: Shared::new(Arc::clone(bytes), range),
        replay: false,
    })
}

/// Splices the store's bytes into `paint` where its images go; a splice whose bytes the
/// store does not hold (dropped over the cap, evicted, or still arriving) is skipped.
pub(in crate::streams) fn painted(paint: FirstPaint, store: &ImageStore) -> Painted {
    let bytes: Arc<[u8]> = Arc::from(paint.bytes.as_slice());
    let mut pieces = Vec::new();
    let mut replayed = Vec::new();
    let mut from = 0;
    for splice in &paint.images {
        let Some(runs) = store.pieces(splice.key) else {
            continue;
        };
        pieces.extend(plain(&bytes, from..splice.at));
        from = splice.at;
        let cursor: Arc<[u8]> = Arc::from(splice.cursor.as_slice());
        pieces.extend(plain(&cursor, 0..cursor.len()));
        pieces.extend(runs.iter().map(|run| Piece {
            bytes: Shared::new(Arc::clone(run), 0..run.len()),
            replay: true,
        }));
        replayed.push(ByteOffset::new(splice.key.get()));
    }
    pieces.extend(plain(&bytes, from..bytes.len()));
    let released = store.released();
    let replay = (!replayed.is_empty() || !released.is_empty())
        .then(|| GraphicsReplayedData::new(replayed, released));
    Painted {
        paint,
        pieces,
        replay,
    }
}

impl TerminalStreams {
    /// The placeholder of every image in `shown` (a screen read's images) whose bytes this
    /// terminal's image store holds now; an evicted image has none (plan todo 85).
    ///
    /// # Errors
    ///
    /// Returns [`StreamError::Poisoned`] when the hub lock is poisoned.
    pub fn image_placeholders(
        &self,
        shown: &[ScreenImage],
    ) -> Result<Vec<GraphicsPlaceholder>, StreamError> {
        Ok(image_placeholders(shown, &self.lock()?.images))
    }
}
