//! Wire records built from core leases and VT paints.

use ovis_terminal_core::leases::{Lease, LeaseSlot};
use ovis_terminal_protocol::schema::common::Geometry;
use ovis_terminal_protocol::schema::ids::{LeaseId, StreamId};
use ovis_terminal_protocol::schema::records::{
    AttachControl, Checkpoint, ControlKind, LeaseRecord, LeaseRole, Modes, PaintedScreen,
    ScreenCursor, TerminalMode,
};
use ovis_terminal_protocol::schema::scalar::{ByteOffset, Count, LeaseEpoch, OpenEnum, Revision};
use ovis_terminal_vt::{
    FirstPaint, KeyEncoding, ModifyOtherKeys, MouseTracking, PasteMode, ScreenBuffer, TerminalModes,
};

use super::offset::StreamOffset;

/// The wire record of a core lease.
pub(super) fn lease_record(lease: &Lease) -> LeaseRecord {
    let role = match lease.slot {
        LeaseSlot::Observe => LeaseRole::Observe,
        _ => LeaseRole::Interact,
    };
    LeaseRecord::new(
        LeaseId::new(lease.lease_id.to_string()),
        LeaseEpoch::new(lease.epoch.get()),
        (role, lease.owner.client.clone()),
    )
}

pub(super) fn enabled(modes: &TerminalModes) -> Vec<OpenEnum<TerminalMode>> {
    let flags = [
        (
            modes.paste == PasteMode::Bracketed,
            TerminalMode::BracketedPaste,
        ),
        (
            modes.screen.buffer == ScreenBuffer::Alternate,
            TerminalMode::AltScreen,
        ),
        (
            modes.mouse.tracking != MouseTracking::Off,
            TerminalMode::Mouse,
        ),
        (
            modes.keyboard.cursor_keys == KeyEncoding::Application,
            TerminalMode::AppCursor,
        ),
        (modes.keyboard.win32_input, TerminalMode::Win32Input),
        (
            modes.keyboard.modify_other_keys != ModifyOtherKeys::Off,
            TerminalMode::ModifyOtherKeys,
        ),
    ];
    flags
        .into_iter()
        .filter(|(on, _)| *on)
        .map(|(_, mode)| OpenEnum::Known(mode))
        .collect()
}

/// The checkpoint record announcing `paint` at stream offset `at`.
pub(super) fn checkpoint(paint: &FirstPaint, at: StreamOffset) -> Checkpoint {
    let mut screen = PaintedScreen::new(
        Revision::new(paint.revision.get()),
        Geometry::new(paint.geometry.columns(), paint.geometry.rows()),
    );
    screen.modes = Modes::new(
        enabled(&paint.modes),
        paint.modes.keyboard.kitty_flags,
        Revision::new(paint.mode_revision.get()),
    );
    screen.cursor = ScreenCursor::new(paint.cursor.row, paint.cursor.column, paint.cursor.visible);
    screen.history_lines = Count::new(u64::from(paint.history_lines));
    Checkpoint::new(ByteOffset::new(paint.offset.get()), at.wire(), screen)
}

/// The `checkpoint` control record that precedes the paint bytes.
pub(super) fn checkpoint_control(stream: &StreamId, checkpoint: Checkpoint) -> AttachControl {
    let mut record = AttachControl::new(
        stream.clone(),
        ControlKind::Checkpoint,
        checkpoint.stream_offset,
    );
    record.checkpoint = Some(checkpoint);
    record
}
