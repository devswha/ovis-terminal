//! Rebuilding a screen from its durable checkpoint and the holder's retained output.
//!
//! Replay never repeats a side effect the crashed core already had: terminal queries
//! completed at or before the answered fence are suppressed by the engine, and OSC
//! records at or before the committed fence are dropped, so status reports are not
//! published twice. Bytes nobody retains are reported as a gap and make the image
//! inexact; they are never filled in.

use ovis_terminal_holder::protocol::Output;
use ovis_terminal_protocol::domain::recovery::{OutputGap, ScreenRecovery};
use ovis_terminal_protocol::schema::scalar::ByteOffset;
use ovis_terminal_vt::{Checkpoint, OscRecord, OutputOffset, TerminalQuery, Vt, VtConfig};

use super::RecoveryError;

/// Side effects the crashed core already performed, by output offset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Fence {
    /// Every query completed at or before this offset was answered.
    pub queries_answered_through: u64,
    /// Every OSC record completed at or before this offset was committed.
    pub osc_committed_through: u64,
}

impl Fence {
    /// Queries answered and OSC records committed through the given offsets.
    #[must_use]
    pub const fn new(queries_answered_through: u64, osc_committed_through: u64) -> Self {
        Self {
            queries_answered_through,
            osc_committed_through,
        }
    }
}

/// Raw output the holder still retains: `bytes` cover `[from, through)`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Retained {
    /// First retained offset.
    pub from: u64,
    /// Offset after the last retained byte.
    pub through: u64,
    /// The bytes.
    pub bytes: Vec<u8>,
}

impl Retained {
    /// `bytes` starting at `from`.
    #[must_use]
    pub fn new(from: u64, bytes: Vec<u8>) -> Self {
        let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        Self {
            from,
            through: from.saturating_add(length),
            bytes,
        }
    }
}

/// A rebuilt screen and what replay produced.
#[derive(Debug)]
#[non_exhaustive]
pub struct Restored {
    /// The engine.
    pub vt: Vt,
    /// How it was rebuilt.
    pub screen: ScreenRecovery,
    /// Queries after the answered fence, located at stream offsets, still owed a reply.
    pub queries: Vec<TerminalQuery>,
    /// OSC records after the committed fence, located at stream offsets.
    pub osc: Vec<OscRecord>,
    /// Bytes of gap: the engine's own offset lags the stream offset by this much.
    pub lag: u64,
}

/// Rebuilds the screen from `checkpoint` (or an empty one built with `config`) and the
/// retained bytes after it.
///
/// # Errors
///
/// [`RecoveryError::Engine`] when the checkpoint cannot be imported, and
/// [`RecoveryError::CheckpointTarget`] when the holder's output ends before it.
pub fn restore(
    (checkpoint, config): (Option<&Checkpoint>, VtConfig),
    retained: &Retained,
    fence: Fence,
) -> Result<Restored, RecoveryError> {
    let mut vt = match checkpoint {
        Some(checkpoint) => Vt::from_checkpoint(checkpoint)?,
        None => Vt::new(config),
    };
    let start = vt.offset().get();
    if retained.through < start {
        return Err(RecoveryError::CheckpointTarget);
    }
    let lag = retained.from.saturating_sub(start);
    let skip = usize::try_from(start.saturating_sub(retained.from)).unwrap_or(usize::MAX);
    let fresh = retained.bytes.get(skip..).unwrap_or_default();
    vt.assume_queries_answered_through(OutputOffset::new(
        fence.queries_answered_through.saturating_sub(lag),
    ));
    let mut fed = vt.feed(fresh);
    for query in &mut fed.queries {
        query.offset = OutputOffset::new(query.offset.get().saturating_add(lag));
    }
    for record in &mut fed.osc {
        record.offset = OutputOffset::new(record.offset.get().saturating_add(lag));
    }
    fed.osc
        .retain(|record| record.offset.get() > fence.osc_committed_through);
    let gaps: Vec<OutputGap> = (lag > 0)
        .then(|| OutputGap::new(ByteOffset::new(start), ByteOffset::new(retained.from)))
        .into_iter()
        .collect();
    let exact = gaps.is_empty() && vt.fidelity().is_exact();
    Ok(Restored {
        screen: ScreenRecovery::new(
            (
                checkpoint.map(|checkpoint| ByteOffset::new(checkpoint.offset().get())),
                ByteOffset::new(start.max(retained.from)),
                ByteOffset::new(retained.through),
            ),
            gaps,
            exact,
        ),
        vt,
        queries: fed.queries,
        osc: fed.osc,
        lag,
    })
}

/// Admits live holder output after a replay, dropping bytes the replay already fed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Splice {
    next: u64,
}

impl Splice {
    /// Live output must continue at `next`, the replay's head.
    #[must_use]
    pub const fn at(next: u64) -> Self {
        Self { next }
    }

    /// The offset the next fresh byte must have.
    #[must_use]
    pub const fn next(self) -> u64 {
        self.next
    }

    /// The fresh suffix of `output`; empty when the replay already covered it.
    ///
    /// # Errors
    ///
    /// [`RecoveryError::HolderProtocol`] when `output` starts after `next`: the holder
    /// skipped bytes on a live link.
    pub fn admit<'a>(&mut self, output: &'a Output) -> Result<&'a [u8], RecoveryError> {
        if output.offset > self.next {
            return Err(RecoveryError::HolderProtocol {
                expected: "contiguous live output",
            });
        }
        let skip = usize::try_from(self.next.saturating_sub(output.offset)).unwrap_or(usize::MAX);
        let fresh = output.bytes.get(skip..).unwrap_or_default();
        self.next = self
            .next
            .saturating_add(u64::try_from(fresh.len()).unwrap_or(u64::MAX));
        Ok(fresh)
    }
}
