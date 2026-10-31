//! Output waits from a cursor captured before the send.

use std::num::NonZeroU32;

use ovis_terminal_vt::OutputOffset;
use serde::{Deserialize, Serialize};

use super::board::Board;
use super::entry::{Armed, Chunk, Condition, Entry, Start, Watch};
use super::error::WaitError;
use super::facts::{ExitEvidence, Facts, ProcessState};
use super::scan::{Scan, Step};
use crate::input::receipts::Receipt;
use crate::screen::{OutputRequest, Screen, ScreenError};

const PAGE: NonZeroU32 = NonZeroU32::MIN.saturating_add(64 * 1024 - 1);

/// Wait for `pattern` in raw output at or after `cursor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputWait {
    cursor: OutputOffset,
    pattern: Vec<u8>,
}

impl OutputWait {
    /// Matches output from `cursor` on, including output already retained.
    ///
    /// # Errors
    ///
    /// Returns [`WaitError::EmptyPattern`] for an empty pattern.
    pub fn new(cursor: OutputOffset, pattern: &[u8]) -> Result<Self, WaitError> {
        if pattern.is_empty() {
            return Err(WaitError::EmptyPattern);
        }
        Ok(Self {
            cursor,
            pattern: pattern.to_vec(),
        })
    }

    /// Matches output from the cursor `receipt` captured at admission, before its first byte.
    ///
    /// # Errors
    ///
    /// Returns [`WaitError::EmptyPattern`] for an empty pattern.
    pub fn since(receipt: &Receipt, pattern: &[u8]) -> Result<Self, WaitError> {
        Self::new(OutputOffset::new(receipt.output_cursor.get()), pattern)
    }
}

/// How an output wait ended; every variant names where a later wait may resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum OutputOutcome {
    /// The pattern starts at `at`; `cursor` is the offset after it.
    Matched {
        /// First byte of the match.
        at: OutputOffset,
        /// Offset after the match.
        cursor: OutputOffset,
    },
    /// The process exited and its final output holds no match.
    Exited {
        /// The holder's exit report.
        exit: ExitEvidence,
        /// Where a later wait may resume.
        cursor: OutputOffset,
    },
    /// The holder link ended; no further output can be observed.
    Lost {
        /// Where a later wait may resume.
        cursor: OutputOffset,
    },
    /// Retention overtook the wait's position after it armed.
    Gap {
        /// First byte the wait had not scanned.
        requested: OutputOffset,
        /// Oldest retained byte.
        floor: OutputOffset,
        /// Offset after the newest byte.
        head: OutputOffset,
    },
    /// The deadline passed without a match; nothing matched.
    Timeout {
        /// Where a later wait may resume.
        cursor: OutputOffset,
    },
    /// The wait's scope was cancelled.
    Cancelled {
        /// Where a later wait may resume.
        cursor: OutputOffset,
    },
}

#[derive(Debug)]
pub(super) struct OutputWatch {
    scan: Scan,
}

fn evicted(error: ScreenError) -> WaitError {
    match error {
        ScreenError::HistoryGap {
            requested,
            retained,
            ..
        } => WaitError::OutputGap {
            requested,
            floor: retained.floor,
            head: retained.head,
        },
        other => WaitError::Screen(other),
    }
}

impl OutputWatch {
    fn matched(&self, at: OutputOffset) -> OutputOutcome {
        OutputOutcome::Matched {
            at,
            cursor: self.scan.after(at),
        }
    }

    /// Scans retained output from the wait's position to the head.
    fn read(&mut self, screen: &Screen) -> Result<Option<OutputOutcome>, WaitError> {
        loop {
            let request = OutputRequest::new(self.scan.position(), PAGE);
            let slice = screen.output(&request).map_err(evicted)?;
            if let Step::Found(at) = self.scan.feed(slice.from, &slice.bytes) {
                return Ok(Some(self.matched(at)));
            }
            if !slice.truncated {
                return Ok(None);
            }
        }
    }

    fn catch_up(&mut self, screen: &Screen) -> Result<Option<OutputOutcome>, WaitError> {
        match self.read(screen) {
            Err(WaitError::OutputGap {
                requested,
                floor,
                head,
            }) => Ok(Some(OutputOutcome::Gap {
                requested,
                floor,
                head,
            })),
            other => other,
        }
    }

    pub(super) fn feed(
        &mut self,
        chunk: Chunk<'_>,
        screen: &Screen,
    ) -> Result<Option<OutputOutcome>, WaitError> {
        match self.scan.feed(chunk.from, chunk.bytes) {
            Step::Found(at) => Ok(Some(self.matched(at))),
            Step::Pending => Ok(None),
            Step::Behind => self.catch_up(screen),
        }
    }
}

impl Watch for OutputWatch {
    type Outcome = OutputOutcome;

    fn settle(&mut self, facts: &Facts) -> Option<OutputOutcome> {
        let cursor = self.scan.resume();
        match facts.process {
            ProcessState::Running => None,
            ProcessState::Exited(exit) => Some(OutputOutcome::Exited { exit, cursor }),
            ProcessState::Lost => Some(OutputOutcome::Lost { cursor }),
        }
    }

    fn expire(&self, _facts: &Facts) -> OutputOutcome {
        OutputOutcome::Timeout {
            cursor: self.scan.resume(),
        }
    }

    fn cancelled(&self, _facts: &Facts) -> OutputOutcome {
        OutputOutcome::Cancelled {
            cursor: self.scan.resume(),
        }
    }
}

impl Condition for OutputWait {
    type Watch = OutputWatch;

    fn start(self, board: &Board) -> Result<Start<OutputWatch>, WaitError> {
        let head = board.screen.live().map_err(WaitError::Screen)?.offset;
        if self.cursor > head {
            return Err(WaitError::CursorAhead {
                cursor: self.cursor,
                head,
            });
        }
        let mut watch = OutputWatch {
            scan: Scan::new(self.pattern, self.cursor),
        };
        if let Some(outcome) = watch.read(&board.screen)? {
            return Ok(Start::Done(outcome));
        }
        Ok(Start::settled(watch, &board.facts))
    }

    fn cancelled(&self, _facts: &Facts) -> OutputOutcome {
        OutputOutcome::Cancelled {
            cursor: self.cursor,
        }
    }

    fn entry(armed: Armed<OutputWatch>) -> Entry {
        Entry::Output(armed)
    }
}
