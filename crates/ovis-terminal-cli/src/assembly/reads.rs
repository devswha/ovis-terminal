//! `terminal.screen`, `terminal.output` and `terminal.wait_output` from the live screen of
//! each incarnation; reads never write to the program or move a viewport.
//!
//! An output wait is armed on the actor thread, in the same order as every holder fact,
//! and awaited off it. A cursor already behind retention when armed is `cursor_expired`;
//! retention overtaking an armed wait is the `gap` outcome.

use std::time::{Duration, Instant};

use base64::Engine as _;
use ovis_terminal_core::screen::OutputRequest;
use ovis_terminal_core::waits::{CancelScope, OutputOutcome, OutputWait, Ticket, WaitError};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::common::{Pattern, TextFormat};
use ovis_terminal_protocol::schema::methods::terminal_read::{
    TerminalOutputParams, TerminalOutputResult, TerminalScreenParams, TerminalScreenResult,
    TerminalWaitOutputParams, TerminalWaitOutputResult,
};
use ovis_terminal_server::streams::wire_modes;
use ovis_terminal_vt::OutputOffset;
use serde_json::{Value, json};

use super::actor::Runtime;
use super::error::{checked, internal, refusal};
use super::input::live_terminal;
use super::records::unreported;

fn wait_refusal(error: &WaitError) -> BoundaryError {
    match error {
        WaitError::OutputGap { .. } => refusal(
            "cursor_expired",
            "the cursor is behind retained output; read terminal.output from its floor",
        ),
        WaitError::EmptyPattern => refusal("invalid_pattern", "name a non-empty pattern"),
        WaitError::CursorAhead { .. } => refusal(
            "invalid_params",
            "the cursor lies after the newest output byte",
        ),
        other => internal("arm an output wait")(other),
    }
}

/// The wire result of a finished output wait.
pub(super) fn waited(outcome: &OutputOutcome) -> Result<Value, BoundaryError> {
    let (kind, matched, cursor) = match outcome {
        OutputOutcome::Matched { at, cursor } => ("matched", Some(at.get().to_string()), *cursor),
        OutputOutcome::Exited { cursor, .. } => ("exited", None, *cursor),
        OutputOutcome::Lost { cursor } => ("lost", None, *cursor),
        OutputOutcome::Gap { requested, .. } => ("gap", None, *requested),
        OutputOutcome::Timeout { cursor } => ("timeout", None, *cursor),
        OutputOutcome::Cancelled { cursor } => ("cancelled", None, *cursor),
        _ => ("timeout", None, OutputOffset::new(0)),
    };
    checked::<TerminalWaitOutputResult>(json!({
        "outcome": kind,
        "matched_at": matched,
        "cursor": {"offset": cursor.get().to_string()},
        "status": unreported(),
    }))
}

impl Runtime {
    pub(super) fn screen(&mut self, params: &TerminalScreenParams) -> Result<Value, BoundaryError> {
        if params.format != TextFormat::Text {
            return Err(refusal("format_unsupported", "request format text"));
        }
        let live = live_terminal((&self.directory, &mut self.live), &params.target)?;
        let view = live
            .hub
            .screen()
            .screen()
            .map_err(|error| refusal(error.kind(), error.hint()))?;
        let lines: Vec<&str> = view.lines.iter().map(|line| line.text.as_str()).collect();
        checked::<TerminalScreenResult>(json!({
            "revision": view.meta.revision.get().to_string(),
            "output_offset": view.meta.offset.get().to_string(),
            "lines": lines,
            "cursor": {"row": view.cursor.row, "col": view.cursor.column, "visible": view.cursor.visible},
            "modes": wire_modes(&view.modes, view.mode_revision),
            "title": view.title,
            "truncated": false,
        }))
    }

    pub(super) fn output(&mut self, params: &TerminalOutputParams) -> Result<Value, BoundaryError> {
        let live = live_terminal((&self.directory, &mut self.live), &params.target)?;
        let limit = u32::try_from(params.limit_bytes.get())
            .ok()
            .and_then(std::num::NonZeroU32::new)
            .ok_or_else(|| {
                refusal(
                    "frame_too_large",
                    "request a positive limit_bytes that fits a frame",
                )
            })?;
        let request = OutputRequest::new(OutputOffset::new(params.from_offset.get()), limit);
        let slice = live
            .hub
            .screen()
            .output(&request)
            .map_err(|error| refusal(error.kind(), error.hint()))?;
        checked::<TerminalOutputResult>(json!({
            "data_b64": base64::engine::general_purpose::STANDARD.encode(&slice.bytes),
            "from_offset": slice.from.get().to_string(),
            "next_offset": slice.next.get().to_string(),
            "floor_offset": slice.retained.floor.get().to_string(),
            "gap": slice.from.get() > params.from_offset.get(),
        }))
    }

    /// Arms the wait in holder order; the caller awaits the ticket off the actor thread.
    pub(super) fn arm_output_wait(
        &mut self,
        params: &TerminalWaitOutputParams,
    ) -> Result<(Ticket<OutputOutcome>, Instant), BoundaryError> {
        let Pattern::Text(text) = &params.pattern else {
            return Err(refusal(
                "invalid_pattern",
                "output waits match literal text patterns",
            ));
        };
        let live = live_terminal((&self.directory, &mut self.live), &params.target)?;
        let wait = OutputWait::new(
            OutputOffset::new(params.cursor.offset.get()),
            text.as_bytes(),
        )
        .map_err(|error| wait_refusal(&error))?;
        let ticket = live
            .waits
            .output(wait, &CancelScope::new())
            .map_err(|error| wait_refusal(&error))?;
        let bound = Duration::from_millis(params.timeout_ms.get());
        let deadline = Instant::now()
            .checked_add(bound)
            .ok_or_else(|| refusal("invalid_params", "timeout_ms is too large"))?;
        Ok((ticket, deadline))
    }
}
