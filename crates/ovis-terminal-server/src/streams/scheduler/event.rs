//! Events a terminal actor publishes after the change they report is committed.

use std::sync::Arc;
use std::time::Instant;

use ovis_terminal_vt::{Geometry, OutputOffset};
use tokio::sync::broadcast;

use super::input::Ticket;

/// One committed control-plane change of a terminal.
///
/// Control events have their own channel, so output volume never pushes them out; a
/// subscriber that falls more than the channel capacity behind is told how many it lost.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TerminalEvent {
    /// An input entry joined the queue.
    Admitted {
        /// The entry.
        ticket: Ticket,
        /// When the actor committed it.
        at: Instant,
    },
    /// The writer started an entry; from now on it can no longer be withdrawn.
    Writing {
        /// The entry.
        ticket: Ticket,
    },
    /// An input entry was written completely.
    Written {
        /// The entry.
        ticket: Ticket,
        /// Bytes written.
        bytes: u64,
    },
    /// A queued entry was withdrawn before any of it was written.
    Withdrawn {
        /// The entry.
        ticket: Ticket,
        /// When the actor committed it.
        at: Instant,
    },
    /// The grid changed size.
    Resized {
        /// The new size.
        geometry: Geometry,
        /// When the actor committed it.
        at: Instant,
    },
}

/// Program output fanned out to viewers: one shared `Arc` per chunk, never a copy each.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum OutputEvent {
    /// Program output accepted for parsing, starting at `offset`.
    Chunk {
        /// Raw output offset of the first byte.
        offset: OutputOffset,
        /// The chunk, shared with every other subscriber.
        bytes: Arc<[u8]>,
    },
    /// A parse turn advanced the screen to `offset`.
    Parsed {
        /// Raw output offset the screen now reflects.
        offset: OutputOffset,
    },
}

/// Sends `event` to every current subscriber; returns whether anyone received it.
pub(super) fn publish<T>(events: &broadcast::Sender<T>, event: T) -> bool {
    events.send(event).is_ok()
}
