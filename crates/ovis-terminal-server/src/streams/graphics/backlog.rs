//! The per-stream backlog bound while whole graphics sequences wait to be sent.
//!
//! Retained graphics a paint re-sends are bounded by the terminal's image store, not
//! charged to the backlog (plan todo 84).

use super::super::limits::StreamLimits;
use super::super::log::OutputLog;
use super::super::viewer::Viewer;

/// Whether `viewer`'s backlog passes its bound, which whole graphics sequences still to
/// send raise by their size, up to one sequence cap.
pub(in crate::streams) fn over_backlog(
    log: &OutputLog,
    viewer: &Viewer,
    limits: StreamLimits,
) -> bool {
    let backlog = viewer.charged_backlog(log);
    backlog > limits.backlog_bytes && {
        let cap = limits.graphics.sequence_max_bytes.get();
        let graphics = log.graphics_after(viewer.cursor()).min(cap);
        backlog > limits.backlog_bytes.saturating_add(graphics)
    }
}
