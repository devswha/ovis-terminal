//! The VT engine: authoritative screen, history, cursor and modes for one
//! terminal, fed with raw output bytes.

mod capture;
mod checkpoint;
mod decimal;
mod error;
mod fidelity;
mod history;
mod images;
mod listener;
mod mirror;
mod modes;
mod observer;
mod osc;
mod paint;
#[cfg(test)]
mod paint_equivalence;
mod paint_rows;
mod paint_state;
mod query;
mod read;
mod recorder;
mod restore;
mod sgr;
mod shadow;
mod tracker;
mod units;

use std::fmt;

use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::{Config as TermConfig, Osc52, Term};
use alacritty_terminal::vte::ansi::{Color, Processor};
pub use checkpoint::{CHECKPOINT_ENGINE, CHECKPOINT_FORMAT, CHECKPOINT_VERSION, Checkpoint};
pub use error::{CheckpointVersion, VtError};
pub use fidelity::{Fidelity, InexactReason};
pub use history::{
    HistoryCompleteness, HistoryRead, HistoryUnavailableReason, LineSpan, SearchMatch,
};
pub use images::{IMAGE_LIMIT, ImagePlace, ImageRelease, ImageSet, ImageSplice, RetainedImage};
pub use modes::{
    EditingModes, KeyEncoding, KeyboardModes, ModifyOtherKeys, MouseEncoding, MouseModes,
    MouseTracking, PasteMode, ScreenBuffer, ScreenModes, TerminalModes,
};
pub use osc::{OSC_FIELD_LIMIT, OSC_RECORD_LIMIT, OscContent, OscFault, OscRecord, OscTerminator};
pub use paint::FirstPaint;
pub use query::{ClientFact, QueryOutcome, TerminalQuery};
pub use read::{
    CursorShape, CursorState, ReadFormat, ReadSource, ScreenImage, ScreenLine, ScreenRead,
};
use serde::Serialize;
pub use units::{
    ContentRevision, Geometry, HistoryLimit, MIN_COLUMNS, ModeRevision, OutputOffset, VtConfig,
};

use self::images::Images;
use self::listener::{ImmediateUpdates, Listener};
use self::query::outcome;
use self::recorder::Recorder;
use self::shadow::Shadow;
use self::tracker::{Cut, Scan, Tracker};

/// Everything one call to [`Vt::feed`] produced.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[non_exhaustive]
pub struct FeedOutcome {
    /// Queries completed in this chunk, in stream order.
    pub queries: Vec<TerminalQuery>,
    /// OSC records completed in this chunk, in stream order.
    pub osc: Vec<OscRecord>,
    /// Queries at or before the ownership fence, already answered elsewhere.
    pub suppressed_queries: u32,
}

struct Size(Geometry);

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }

    fn screen_lines(&self) -> usize {
        self.0.row_count()
    }

    fn columns(&self) -> usize {
        self.0.column_count()
    }
}

/// Terminal state engine over `alacritty_terminal`.
pub struct Vt {
    term: Term<Listener>,
    listener: Listener,
    processor: Processor<ImmediateUpdates>,
    shadow: Shadow,
    inactive: Grid<Cell>,
    tracker: Tracker,
    images: Images,
    config: VtConfig,
    offset: OutputOffset,
    revision: ContentRevision,
    mode_revision: ModeRevision,
    modes: TerminalModes,
    fidelity: Fidelity,
    restored_at: Option<OutputOffset>,
    queries_owned_through: OutputOffset,
}

impl fmt::Debug for Vt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vt")
            .field("config", &self.config)
            .field("offset", &self.offset)
            .field("revision", &self.revision)
            .field("mode_revision", &self.mode_revision)
            .field("fidelity", &self.fidelity)
            .finish_non_exhaustive()
    }
}

impl Vt {
    /// Creates an engine at offset zero with a blank screen.
    pub fn new(config: VtConfig) -> Self {
        let listener = Listener::default();
        let term_config = TermConfig {
            scrolling_history: config.history_limit.line_count(),
            kitty_keyboard: true,
            osc52: Osc52::Disabled,
            ..TermConfig::default()
        };
        let term = Term::new(term_config, &Size(config.geometry), listener.clone());
        let shadow = Shadow::new(config.geometry);
        let modes = TerminalModes::capture(*term.mode(), &shadow);
        Self {
            inactive: Grid::new(
                config.geometry.row_count(),
                config.geometry.column_count(),
                0,
            ),
            term,
            listener,
            processor: Processor::new(),
            shadow,
            tracker: Tracker::new(),
            images: Images::default(),
            config,
            offset: OutputOffset::ZERO,
            revision: ContentRevision::default(),
            mode_revision: ModeRevision::default(),
            modes,
            fidelity: Fidelity::Exact,
            restored_at: None,
            queries_owned_through: OutputOffset::ZERO,
        }
    }

    /// Consumes output bytes; chunk boundaries never change the result.
    pub fn feed(&mut self, bytes: &[u8]) -> FeedOutcome {
        let start = self.offset;
        let Scan {
            forward,
            cuts,
            completions,
            osc,
        } = self.tracker.scan(start, bytes);
        let mut result = FeedOutcome {
            osc,
            ..FeedOutcome::default()
        };
        let end = Cut {
            forward_end: forward.len(),
            input_end: bytes.len(),
            image: None,
        };
        let mut from = 0;
        for cut in cuts.into_iter().chain(std::iter::once(end)) {
            if let Some(segment) = forward
                .get(from..cut.forward_end)
                .filter(|part| !part.is_empty())
            {
                let offset = start.advanced_by(cut.input_end);
                for answer in self.apply(segment) {
                    if offset <= self.queries_owned_through {
                        result.suppressed_queries = result.suppressed_queries.saturating_add(1);
                    } else {
                        result.queries.push(TerminalQuery {
                            offset,
                            outcome: answer,
                        });
                    }
                }
            }
            from = cut.forward_end;
            if let Some(seen) = cut.image {
                self.receive_image(seen);
            }
        }
        self.follow_images();
        self.offset = start.advanced_by(bytes.len());
        self.revision = self.revision.advanced_by(completions);
        result
    }

    fn apply(&mut self, segment: &[u8]) -> Vec<QueryOutcome> {
        let mut replies = Vec::new();
        let mut recorder = Recorder {
            term: &mut self.term,
            shadow: &mut self.shadow,
            inactive: &mut self.inactive,
            geometry: self.config.geometry,
            replies: &mut replies,
        };
        self.processor.advance(&mut recorder, segment);
        self.refresh_modes();
        let colors = self.term.colors();
        let raised = self
            .listener
            .drain()
            .into_iter()
            .filter_map(|event| outcome(event, colors));
        replies
            .into_iter()
            .map(QueryOutcome::Reply)
            .chain(raised)
            .collect()
    }

    fn refresh_modes(&mut self) {
        let modes = TerminalModes::capture(*self.term.mode(), &self.shadow);
        if modes != self.modes {
            self.modes = modes;
            self.mode_revision = self.mode_revision.next();
        }
    }

    /// Changes the grid size, reflowing the primary screen.
    pub fn resize(&mut self, geometry: Geometry) {
        if geometry == self.config.geometry {
            return;
        }
        let alternate = self.modes.screen.buffer == ScreenBuffer::Alternate;
        self.inactive
            .resize::<Color>(alternate, geometry.row_count(), geometry.column_count());
        self.term.resize(Size(geometry));
        self.shadow.resize(geometry);
        self.config.geometry = geometry;
        self.revision = self.revision.advanced_by(1);
        self.refresh_modes();
        self.follow_images();
    }

    /// Declares that every query completed at or before `offset` was already
    /// answered (by the engine instance this one replaces), so replaying
    /// retained output does not answer it twice.
    pub const fn assume_queries_answered_through(&mut self, offset: OutputOffset) {
        self.queries_owned_through = offset;
    }

    /// Bytes consumed so far.
    pub const fn offset(&self) -> OutputOffset {
        self.offset
    }

    /// Content revision.
    pub const fn revision(&self) -> ContentRevision {
        self.revision
    }

    /// Mode revision.
    pub const fn mode_revision(&self) -> ModeRevision {
        self.mode_revision
    }

    /// Structured terminal modes.
    pub const fn modes(&self) -> TerminalModes {
        self.modes
    }

    /// Current grid size.
    pub const fn geometry(&self) -> Geometry {
        self.config.geometry
    }

    /// Whether this engine is an exact image of its output stream.
    pub const fn fidelity(&self) -> &Fidelity {
        &self.fidelity
    }
}
