//! The segment store: append, retain, read by offset, reopen.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};

use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_vt::OutputOffset;

use super::error::HistoryError;

const SUFFIX: &str = ".seg";

/// Segment size and retention bound of one terminal's history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryConfig {
    segment: u64,
    retained: u64,
}

impl HistoryConfig {
    /// `retained` bytes in segments of `segment` bytes.
    ///
    /// # Errors
    ///
    /// Returns `invalid_params` when `segment` is zero or larger than `retained`.
    pub const fn new(segment: u64, retained: u64) -> Result<Self, HistoryError> {
        if segment == 0 || segment > retained {
            return Err(HistoryError::InvalidConfig { segment, retained });
        }
        Ok(Self { segment, retained })
    }

    /// The advertised output ring (64 MiB) in 16 segments.
    #[must_use]
    pub fn advertised(limits: &Limits) -> Self {
        let retained = limits.output_ring_bytes.get().max(16);
        Self {
            segment: retained / 16,
            retained,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Segment {
    start: u64,
    len: u64,
}

impl Segment {
    const fn end(self) -> u64 {
        self.start.saturating_add(self.len)
    }
}

/// Bytes read from the history, with the window they were read from.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct HistoryRead {
    /// Offset of the first returned byte.
    pub offset: OutputOffset,
    /// First offset still readable.
    pub floor: OutputOffset,
    /// Offset after the last recorded byte.
    pub end: OutputOffset,
    /// The bytes, at most the requested count.
    pub bytes: Vec<u8>,
}

/// The state of one history, measured from its files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct HistoryStats {
    /// First offset still readable.
    pub floor: u64,
    /// Offset after the last recorded byte.
    pub end: u64,
    /// Segment files on disk.
    pub segments: usize,
    /// Segment files deleted by retention.
    pub evicted_segments: u64,
    /// Bytes the segment files occupy, read from their metadata.
    pub disk_bytes: u64,
    /// The append failure that stopped recording, if any.
    pub fault: Option<std::io::ErrorKind>,
}

/// One terminal's output history in a directory of its own.
#[derive(Debug)]
pub struct HistoryStore {
    dir: PathBuf,
    config: HistoryConfig,
    segments: VecDeque<Segment>,
    current: Option<File>,
    evicted: u64,
    fault: Option<std::io::ErrorKind>,
}

impl HistoryStore {
    /// Opens (creating) the history in `dir`, rebuilding the index from its segments.
    ///
    /// # Errors
    ///
    /// Returns `history_unavailable` when the directory cannot be read or its segments
    /// are not one contiguous stream.
    pub fn open(dir: &Path, config: HistoryConfig) -> Result<Self, HistoryError> {
        fs::create_dir_all(dir).map_err(HistoryError::io("create the directory"))?;
        let mut found = Vec::new();
        for entry in fs::read_dir(dir).map_err(HistoryError::io("list the directory"))? {
            let entry = entry.map_err(HistoryError::io("list the directory"))?;
            let name = entry.file_name();
            let Some(start) = name.to_str().and_then(parse_name) else {
                continue;
            };
            let len = entry
                .metadata()
                .map_err(HistoryError::io("stat a segment"))?
                .len();
            found.push(Segment { start, len });
        }
        found.sort_by_key(|segment| segment.start);
        for pair in found.windows(2) {
            if let [before, after] = pair
                && before.end() != after.start
            {
                return Err(HistoryError::Discontinuous {
                    start: OutputOffset::new(after.start),
                    expected: OutputOffset::new(before.end()),
                });
            }
        }
        let mut store = Self {
            dir: dir.to_path_buf(),
            config,
            segments: found.into(),
            current: None,
            evicted: 0,
            fault: None,
        };
        store.retain()?;
        Ok(store)
    }

    /// Appends `bytes` after the recorded end, then evicts beyond the retention bound.
    ///
    /// # Errors
    ///
    /// Returns the file failure; the store then refuses every later append.
    pub fn append(&mut self, bytes: &[u8]) -> Result<(), HistoryError> {
        if let Some(kind) = self.fault {
            return Err(HistoryError::Faulted(kind));
        }
        let appended = self.write(bytes).and_then(|()| self.retain());
        if let Err(HistoryError::Io { source, .. }) = &appended {
            self.fault = Some(source.kind());
        }
        appended
    }

    fn write(&mut self, mut bytes: &[u8]) -> Result<(), HistoryError> {
        while !bytes.is_empty() {
            let full = self
                .segments
                .back()
                .is_none_or(|last| last.len >= self.config.segment);
            if full || self.current.is_none() {
                self.rotate(full)?;
            }
            let room = self
                .segments
                .back()
                .map_or(0, |last| self.config.segment.saturating_sub(last.len));
            let take = usize::try_from(room).unwrap_or(usize::MAX).min(bytes.len());
            let (now, rest) = bytes.split_at(take);
            if let Some(file) = self.current.as_mut() {
                file.write_all(now)
                    .map_err(HistoryError::io("append to a segment"))?;
            }
            if let Some(last) = self.segments.back_mut() {
                last.len = last
                    .len
                    .saturating_add(u64::try_from(take).unwrap_or(u64::MAX));
            }
            bytes = rest;
        }
        Ok(())
    }

    /// Opens the last segment for appending, or starts a new one when it is full.
    fn rotate(&mut self, full: bool) -> Result<(), HistoryError> {
        if full {
            let start = self.end();
            File::create_new(self.path(start)).map_err(HistoryError::io("create a segment"))?;
            self.segments.push_back(Segment { start, len: 0 });
        }
        let start = self.segments.back().map_or(0, |last| last.start);
        let file = OpenOptions::new()
            .append(true)
            .open(self.path(start))
            .map_err(HistoryError::io("open a segment"))?;
        self.current = Some(file);
        Ok(())
    }

    /// Deletes the oldest segments while the rest still hold the retention bound.
    fn retain(&mut self) -> Result<(), HistoryError> {
        while self.segments.len() > 1 {
            let Some(oldest) = self.segments.front().copied() else {
                break;
            };
            let retained = self.end().saturating_sub(oldest.start);
            if retained <= self.config.retained {
                break;
            }
            fs::remove_file(self.path(oldest.start))
                .map_err(HistoryError::io("delete an evicted segment"))?;
            self.segments.pop_front();
            self.evicted = self.evicted.saturating_add(1);
        }
        Ok(())
    }

    /// Reads up to `max` bytes starting at `offset`.
    ///
    /// # Errors
    ///
    /// Returns `history_gap` below the floor, `invalid_params` past the end and
    /// `history_unavailable` when a segment cannot be read.
    pub fn read(&self, offset: OutputOffset, max: usize) -> Result<HistoryRead, HistoryError> {
        let (floor, end) = (self.floor(), self.end());
        if offset.get() < floor {
            let floor = OutputOffset::new(floor);
            return Err(HistoryError::Evicted { offset, floor });
        }
        if offset.get() > end {
            let end = OutputOffset::new(end);
            return Err(HistoryError::Beyond { offset, end });
        }
        let wanted = u64::try_from(max).unwrap_or(u64::MAX);
        let until = end.min(offset.get().saturating_add(wanted));
        let mut bytes = Vec::new();
        for segment in self
            .segments
            .iter()
            .filter(|s| s.end() > offset.get() && s.start < until)
        {
            let from = offset.get().max(segment.start);
            let to = until.min(segment.end());
            let mut file = File::open(self.path(segment.start))
                .map_err(HistoryError::io("open a segment for reading"))?;
            file.seek(SeekFrom::Start(from.saturating_sub(segment.start)))
                .map_err(HistoryError::io("seek in a segment"))?;
            file.take(to.saturating_sub(from))
                .read_to_end(&mut bytes)
                .map_err(HistoryError::io("read a segment"))?;
        }
        let (floor, end) = (OutputOffset::new(floor), OutputOffset::new(end));
        Ok(HistoryRead {
            offset,
            floor,
            end,
            bytes,
        })
    }

    /// Floor, end and the bytes the segment files occupy on disk.
    ///
    /// # Errors
    ///
    /// Returns `history_unavailable` when a segment cannot be inspected.
    pub fn stats(&self) -> Result<HistoryStats, HistoryError> {
        let mut disk_bytes = 0_u64;
        for segment in &self.segments {
            let len = fs::metadata(self.path(segment.start))
                .map_err(HistoryError::io("stat a segment"))?
                .len();
            disk_bytes = disk_bytes.saturating_add(len);
        }
        Ok(HistoryStats {
            floor: self.floor(),
            end: self.end(),
            segments: self.segments.len(),
            evicted_segments: self.evicted,
            disk_bytes,
            fault: self.fault,
        })
    }

    fn floor(&self) -> u64 {
        self.segments.front().map_or(0, |first| first.start)
    }

    fn end(&self) -> u64 {
        self.segments.back().map_or(0, |last| last.end())
    }

    fn path(&self, start: u64) -> PathBuf {
        self.dir.join(format!("{start:020}{SUFFIX}"))
    }
}

fn parse_name(name: &str) -> Option<u64> {
    let digits = name.strip_suffix(SUFFIX)?;
    (digits.len() == 20 && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}
