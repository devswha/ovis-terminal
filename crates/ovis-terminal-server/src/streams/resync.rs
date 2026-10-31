//! Readiness revocation and gap-plus-checkpoint resynchronization of one viewer.

use std::sync::Arc;

use ovis_terminal_protocol::schema::records::{
    AttachControl, Checkpoint, ControlKind, SkippedOutput,
};
use ovis_terminal_protocol::schema::scalar::ByteOffset;

use super::hub::Painted;
use super::log::OutputLog;
use super::records;
use super::viewer::Viewer;

/// Queues `input_revoked` for an interact viewer, at most once.
pub(super) fn revoke_input(viewer: &mut Viewer) {
    if viewer.interact && !viewer.is_closed() {
        viewer.interact = false;
        let at = viewer.next_offset().wire();
        viewer.control(AttachControl::new(
            viewer.id.clone(),
            ControlKind::InputRevoked,
            at,
        ));
    }
}

/// Replaces a viewer's backlog with a gap record (when bytes were skipped) and a checkpoint.
pub(super) fn resync(log: &OutputLog, viewer: &mut Viewer, painted: &Painted) -> Checkpoint {
    let skipped_from = (viewer.cursor() < log.end()).then(|| log.raw_at(viewer.cursor()));
    viewer.drop_backlog();
    let at = viewer.next_offset();
    if let Some(from) = skipped_from {
        let mut gap = AttachControl::new(viewer.id.clone(), ControlKind::Gap, at.wire());
        gap.skipped = Some(SkippedOutput::new(
            ByteOffset::new(from.get()),
            ByteOffset::new(painted.paint.offset.get()),
        ));
        viewer.control(gap);
    }
    let checkpoint = records::checkpoint(&painted.paint, at);
    let record = records::checkpoint_control(&viewer.id, checkpoint.clone());
    viewer.paint(record, (Arc::clone(&painted.bytes), log.end()));
    checkpoint
}
