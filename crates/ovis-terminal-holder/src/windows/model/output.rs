//! Bounded raw output by absolute offset, with exit evidence behind the last byte.

use std::collections::VecDeque;
use std::num::NonZeroUsize;

use thiserror::Error;

use super::{Ended, OutputEnd, OutputOffset, ProgramExit};

/// The newest `capacity` output bytes, the offset after them, and the exit once known.
///
/// A reader at offset `n` sees bytes from `n` (or the retained floor, flagged as a gap);
/// only a reader that has consumed everything up to the final offset sees the exit, so
/// the exit can never overtake output.
#[derive(Debug)]
pub struct OutputLog {
    retained: VecDeque<u8>,
    capacity: usize,
    head: OutputOffset,
    ended: Option<Ended>,
}

/// One read of an [`OutputLog`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutputRead {
    /// Contiguous bytes starting at `from`.
    Bytes {
        /// Offset of the first byte; above the requested offset when `gap` is set.
        from: OutputOffset,
        /// Raw bytes, never decoded.
        bytes: Vec<u8>,
        /// The requested offset was evicted before it was read.
        gap: bool,
    },
    /// Nothing after the requested offset yet, and the program has not ended.
    Pending,
    /// Everything up to the final offset was read; this is the exit.
    Ended(Ended),
}

/// A request the output log cannot satisfy.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutputError {
    /// The offset is past the newest byte.
    #[error("offset {requested} is past the output head {head}")]
    BeyondHead {
        /// Requested offset.
        requested: u64,
        /// Offset after the newest byte.
        head: u64,
    },
    /// Output was appended after the exit was recorded.
    #[error("the output already ended")]
    AfterEnd,
    /// The offset cannot advance further.
    #[error("the output offset cannot advance further")]
    Exhausted,
}

impl OutputError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::BeyondHead { .. } => "offset_beyond_head",
            Self::AfterEnd => "output_ended",
            Self::Exhausted => "counter_exhausted",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::BeyondHead { .. } => "read from an offset at or below the output head",
            Self::AfterEnd => "report the holder defect; the exit was already published",
            Self::Exhausted => "relaunch the terminal; its offset space is used up",
        }
    }
}

impl OutputLog {
    /// An empty log retaining at most `capacity` bytes.
    #[must_use]
    pub const fn new(capacity: NonZeroUsize) -> Self {
        Self {
            retained: VecDeque::new(),
            capacity: capacity.get(),
            head: OutputOffset::ZERO,
            ended: None,
        }
    }

    /// Offset just past the newest byte.
    #[must_use]
    pub const fn head(&self) -> OutputOffset {
        self.head
    }

    /// Appends `chunk` and returns the offset of its first byte.
    ///
    /// # Errors
    ///
    /// [`OutputError::AfterEnd`] once the exit is recorded, and
    /// [`OutputError::Exhausted`] when the offset would overflow.
    pub fn append(&mut self, chunk: &[u8]) -> Result<OutputOffset, OutputError> {
        if self.ended.is_some() {
            return Err(OutputError::AfterEnd);
        }
        let start = self.head;
        let length = u64::try_from(chunk.len()).map_err(|_| OutputError::Exhausted)?;
        let head = start
            .get()
            .checked_add(length)
            .ok_or(OutputError::Exhausted)?;
        self.head = OutputOffset::new(head);
        self.retained.extend(chunk);
        let excess = self.retained.len().saturating_sub(self.capacity);
        self.retained.drain(..excess);
        Ok(start)
    }

    /// Records the exit at the current head and returns the recorded exit; the first
    /// record stays. It becomes readable after the last byte.
    pub fn end(&mut self, exit: ProgramExit, output: OutputEnd) -> Ended {
        let head = self.head;
        *self.ended.get_or_insert(Ended {
            exit,
            output,
            final_offset: head,
        })
    }

    /// The recorded exit, once known.
    #[must_use]
    pub const fn ended(&self) -> Option<Ended> {
        self.ended
    }

    /// Reads at most `limit` bytes from `from`.
    ///
    /// # Errors
    ///
    /// [`OutputError::BeyondHead`] when `from` is past the newest byte.
    pub fn read(&self, from: OutputOffset, limit: NonZeroUsize) -> Result<OutputRead, OutputError> {
        if from > self.head {
            return Err(OutputError::BeyondHead {
                requested: from.get(),
                head: self.head.get(),
            });
        }
        if from == self.head {
            return Ok(self.ended.map_or(OutputRead::Pending, OutputRead::Ended));
        }
        let retained = u64::try_from(self.retained.len()).map_err(|_| OutputError::Exhausted)?;
        let floor = self.head.get().saturating_sub(retained);
        let start = from.get().max(floor);
        let skip =
            usize::try_from(start.saturating_sub(floor)).map_err(|_| OutputError::Exhausted)?;
        Ok(OutputRead::Bytes {
            from: OutputOffset::new(start),
            bytes: self
                .retained
                .iter()
                .skip(skip)
                .take(limit.get())
                .copied()
                .collect(),
            gap: from.get() < floor,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use proptest::prelude::*;

    use super::{OutputLog, OutputRead};
    use crate::windows::model::{OutputEnd, OutputOffset, ProgramExit};

    fn size(value: usize) -> NonZeroUsize {
        NonZeroUsize::new(value).expect("nonzero")
    }

    fn drain(log: &OutputLog, limit: NonZeroUsize) -> (Vec<u8>, bool, Option<u64>) {
        let (mut seen, mut gap, mut at) = (Vec::new(), false, OutputOffset::ZERO);
        loop {
            match log.read(at, limit).expect("read") {
                OutputRead::Bytes {
                    from,
                    bytes,
                    gap: evicted,
                } => {
                    gap |= evicted;
                    let length = u64::try_from(bytes.len()).expect("len");
                    at = OutputOffset::new(from.get().checked_add(length).expect("offset"));
                    seen.extend(bytes);
                }
                OutputRead::Pending => return (seen, gap, None),
                OutputRead::Ended(ended) => return (seen, gap, Some(ended.final_offset.get())),
            }
        }
    }

    proptest! {
        #[test]
        fn the_exit_is_reached_only_after_every_retained_byte_in_order(
            chunks in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..48), 0..10),
            capacity in 1_usize..96,
            limit in 1_usize..64,
        ) {
            let mut log = OutputLog::new(size(capacity));
            let mut stream = Vec::new();
            for chunk in &chunks {
                prop_assert_eq!(log.append(chunk).expect("append").get(), u64::try_from(stream.len()).expect("len"));
                stream.extend_from_slice(chunk);
            }
            let (before, _, pending_exit) = drain(&log, size(limit));
            prop_assert_eq!(pending_exit, None, "no exit before it is recorded");
            log.end(ProgramExit::Code(0), OutputEnd::Eof);
            let (seen, gap, final_offset) = drain(&log, size(limit));
            let floor = stream.len().saturating_sub(capacity);
            prop_assert_eq!(&seen, stream.get(floor..).expect("suffix"));
            prop_assert_eq!(&before, &seen);
            prop_assert_eq!(gap, floor > 0);
            prop_assert_eq!(final_offset, Some(u64::try_from(stream.len()).expect("len")));
        }
    }

    #[test]
    fn nothing_is_appended_or_ended_twice_and_reads_past_the_head_are_refused() {
        let mut log = OutputLog::new(size(8));
        log.append(b"abc").expect("append");
        assert_eq!(
            log.read(OutputOffset::new(4), size(1))
                .map_err(|error| error.kind()),
            Err("offset_beyond_head")
        );
        log.end(ProgramExit::Code(3), OutputEnd::Eof);
        assert_eq!(
            log.append(b"late").map_err(|error| error.kind()),
            Err("output_ended")
        );
        assert_eq!(
            log.end(ProgramExit::Code(4), OutputEnd::Eof).exit,
            ProgramExit::Code(3)
        );
        let Ok(OutputRead::Ended(ended)) = log.read(OutputOffset::new(3), size(1)) else {
            panic!("the first exit stays recorded");
        };
        assert_eq!(
            (ended.exit, ended.final_offset.get()),
            (ProgramExit::Code(3), 3)
        );
    }
}
