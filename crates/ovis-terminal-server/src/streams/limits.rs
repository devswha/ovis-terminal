//! Per-stream bounds and the session-wide backlog budget.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use ovis_terminal_protocol::domain::graphics::GraphicsCaps;
use ovis_terminal_protocol::schema::ids::StreamId;
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_protocol::schema::scalar::ByteCount;

use super::error::StreamError;

/// Bounds every stream of one runtime obeys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamLimits {
    pub(super) chunk_bytes: usize,
    pub(super) credit_window: u64,
    pub(super) backlog_bytes: u64,
    pub(super) stall_after: Duration,
    pub(super) max_streams: usize,
    pub(super) graphics: GraphicsCaps,
}

impl StreamLimits {
    /// The advertised limits: 16 KiB chunks, 256 KiB credit, a 4 MiB backlog, 30 s
    /// before an unwritable stream closes, 64 streams per terminal and 8 MiB graphics
    /// sequences.
    #[must_use]
    pub fn advertised(limits: &Limits) -> Self {
        let size = |count: u64| usize::try_from(count).unwrap_or(usize::MAX);
        Self {
            chunk_bytes: size(limits.stream_chunk_bytes.get()),
            credit_window: limits.credit_initial_bytes.get(),
            backlog_bytes: limits.stream_backlog_bytes.get(),
            stall_after: Duration::from_secs(30),
            max_streams: 64,
            graphics: GraphicsCaps::advertised(limits),
        }
    }

    /// Replaces the largest graphics sequence a stream passes whole.
    #[must_use]
    pub const fn with_graphics_sequence_max_bytes(mut self, bytes: u64) -> Self {
        self.graphics.sequence_max_bytes = ByteCount::new(bytes);
        self
    }

    /// Replaces the per-stream backlog bound.
    #[must_use]
    pub const fn with_backlog_bytes(self, bytes: u64) -> Self {
        Self {
            backlog_bytes: bytes,
            ..self
        }
    }

    /// Replaces the output credit a stream opens with.
    #[must_use]
    pub const fn with_credit_window(self, bytes: u64) -> Self {
        Self {
            credit_window: bytes,
            ..self
        }
    }

    /// Replaces the time an unwritable stream may stall before it closes.
    #[must_use]
    pub const fn with_stall_after(self, stall_after: Duration) -> Self {
        Self {
            stall_after,
            ..self
        }
    }

    /// Replaces the number of streams one terminal serves.
    #[must_use]
    pub const fn with_max_streams(self, max_streams: usize) -> Self {
        Self {
            max_streams,
            ..self
        }
    }

    /// Output credit a stream opens with.
    #[must_use]
    pub const fn credit_window(&self) -> u64 {
        self.credit_window
    }
}

/// Backlog references of every stream in the session, bounded as one budget.
#[derive(Debug)]
pub struct SessionBacklog {
    cap: u64,
    streams: Mutex<HashMap<StreamId, u64>>,
}

impl SessionBacklog {
    /// A budget of `cap` bytes across all streams.
    #[must_use]
    pub fn new(cap: u64) -> Self {
        Self {
            cap,
            streams: Mutex::new(HashMap::new()),
        }
    }

    /// Records `stream`'s backlog and returns the bytes the session is over budget.
    pub(super) fn set(&self, stream: &StreamId, bytes: u64) -> Result<u64, StreamError> {
        let mut streams = self.streams.lock().map_err(|_| StreamError::Poisoned)?;
        if bytes == 0 {
            streams.remove(stream);
        } else {
            streams.insert(stream.clone(), bytes);
        }
        let total: u64 = streams.values().copied().sum();
        drop(streams);
        Ok(total.saturating_sub(self.cap))
    }

    /// Bytes referenced by every stream's backlog.
    ///
    /// # Errors
    ///
    /// Returns [`StreamError::Poisoned`] when the budget lock is poisoned.
    pub fn total(&self) -> Result<u64, StreamError> {
        Ok(self
            .streams
            .lock()
            .map_err(|_| StreamError::Poisoned)?
            .values()
            .copied()
            .sum())
    }
}
