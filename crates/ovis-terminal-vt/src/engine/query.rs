//! Terminal queries the program asked, with the reply the core owns writing.

use alacritty_terminal::event::Event;
use alacritty_terminal::term::color::{COUNT, Colors};
use serde::{Deserialize, Serialize};

use crate::engine::units::OutputOffset;

/// A fact only the attached client knows, so the engine cannot answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ClientFact {
    /// The program asked for a palette or dynamic color the program never set.
    PaletteColor {
        /// alacritty color index (0-255 palette, 256+ dynamic colors).
        index: u16,
    },
    /// The program asked for the text area in pixels.
    CellPixelSize,
}

/// What the owner must do about one query.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QueryOutcome {
    /// Write these bytes to the program, exactly once.
    Reply(Vec<u8>),
    /// The answer depends on a client fact the engine does not have.
    NeedsClientFact(ClientFact),
}

/// One query, located just past the byte that completed it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TerminalQuery {
    /// Offset just past the query's final byte.
    pub offset: OutputOffset,
    /// Required action.
    pub outcome: QueryOutcome,
}

pub(super) fn outcome(event: Event, colors: &Colors) -> Option<QueryOutcome> {
    match event {
        Event::PtyWrite(text) => Some(QueryOutcome::Reply(text.into_bytes())),
        Event::ColorRequest(index, format) => {
            let known = (index < COUNT).then(|| colors[index]).flatten();
            known.map_or_else(
                || {
                    u16::try_from(index).ok().map(|index| {
                        QueryOutcome::NeedsClientFact(ClientFact::PaletteColor { index })
                    })
                },
                |color| Some(QueryOutcome::Reply(format(color).into_bytes())),
            )
        }
        Event::TextAreaSizeRequest(_) => {
            Some(QueryOutcome::NeedsClientFact(ClientFact::CellPixelSize))
        }
        // OSC 52 is disabled in the emulator configuration, so clipboard events
        // cannot occur; the rest are presentation notifications.
        Event::ClipboardStore(..)
        | Event::ClipboardLoad(..)
        | Event::MouseCursorDirty
        | Event::Title(_)
        | Event::ResetTitle
        | Event::CursorBlinkingChange
        | Event::Wakeup
        | Event::Bell
        | Event::Exit
        | Event::ChildExit(_) => None,
    }
}
