//! The input writer: drains one terminal's queue into the program, one entry at a time.
//!
//! It runs as its own task, so a program that never reads stdin stalls only this task;
//! the actor keeps serving control and parse turns. Reservations return to their budgets
//! once an entry is fully written.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncWrite, AsyncWriteExt as _};
use tokio::sync::{Notify, broadcast};

use super::event::{TerminalEvent, publish};
use super::input::{InputQueue, Next};

#[derive(Debug)]
pub(super) struct Shared {
    pub queue: Mutex<InputQueue>,
    pub wake: Notify,
}

/// Bytes the writer delivered before it stopped, and why it stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct WriterReport {
    /// Entries written completely.
    pub entries: u64,
    /// Bytes written.
    pub bytes: u64,
    /// The program's input failed; nothing after it was written.
    pub failed: bool,
}

pub(super) async fn drain<W: AsyncWrite + Unpin>(
    shared: Arc<Shared>,
    mut sink: W,
    events: broadcast::Sender<TerminalEvent>,
) -> WriterReport {
    let mut report = WriterReport::default();
    loop {
        let next = shared
            .queue
            .lock()
            .map_or(Next::Closed, |mut queue| queue.take());
        let entry = match next {
            Next::Write(entry) => entry,
            Next::Idle => {
                shared.wake.notified().await;
                continue;
            }
            Next::Closed => return report,
        };
        publish(
            &events,
            TerminalEvent::Writing {
                ticket: entry.ticket,
            },
        );
        let written = sink.write_all(&entry.bytes).await.and(sink.flush().await);
        if let Ok(mut queue) = shared.queue.lock() {
            queue.written();
        }
        drop(entry.held);
        if written.is_err() {
            report.failed = true;
            return report;
        }
        let len = u64::try_from(entry.bytes.len()).unwrap_or(u64::MAX);
        report.entries = report.entries.saturating_add(1);
        report.bytes = report.bytes.saturating_add(len);
        publish(
            &events,
            TerminalEvent::Written {
                ticket: entry.ticket,
                bytes: len,
            },
        );
    }
}
