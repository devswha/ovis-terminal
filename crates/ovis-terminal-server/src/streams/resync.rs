//! Readiness revocation and gap-plus-checkpoint resynchronization of one viewer.

use ovis_terminal_core::health::Health;
use ovis_terminal_core::leases::Owner;
use ovis_terminal_protocol::domain::health::{ConsumerRef, LossRange};
use ovis_terminal_protocol::schema::records::{
    AttachControl, Checkpoint, ControlKind, SkippedOutput,
};
use ovis_terminal_protocol::schema::scalar::ByteOffset;

use super::graphics::Painted;
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
    let unsent = viewer.backlog(log);
    viewer.drop_backlog();
    let at = viewer.next_offset();
    let lost = skipped_from.and_then(|from| {
        let mut gap = AttachControl::new(viewer.id.clone(), ControlKind::Gap, at.wire());
        gap.skipped = Some(SkippedOutput::new(
            ByteOffset::new(from.get()),
            ByteOffset::new(painted.paint.offset.get()),
        ));
        viewer.control(gap);
        skipped(from.get(), painted.paint.offset.get())
    });
    let checkpoint = records::checkpoint(&painted.paint, at);
    let record = records::checkpoint_control(&viewer.id, checkpoint.clone());
    viewer.paint(record, (&painted.pieces, log.end()));
    painted.announce(viewer);
    viewer.skip_notices(log.notice_seq());
    viewer.report_resync(unsent, lost);
    viewer.report_queued(viewer.backlog(log));
    checkpoint
}

/// Starts reporting `viewer` to `health` as stream `consumer` of `owner`'s client; a
/// viewer the bounded table refuses runs untracked and is counted as such.
pub(super) fn track(
    viewer: &mut Viewer,
    health: Option<&Health>,
    (owner, consumer): (&Owner, ConsumerRef),
) {
    if let Some(health) = health
        && let Ok(gauge) = health.consumers().track(owner.client.clone(), consumer)
    {
        viewer.report_to(gauge);
    }
}

/// The output bytes `from..until` a gap skipped, as an inclusive range; none when empty.
const fn skipped(from: u64, until: u64) -> Option<LossRange> {
    if until > from {
        Some(LossRange::Bytes {
            first_offset: ByteOffset::new(from),
            last_offset: ByteOffset::new(until.saturating_sub(1)),
        })
    } else {
        None
    }
}
