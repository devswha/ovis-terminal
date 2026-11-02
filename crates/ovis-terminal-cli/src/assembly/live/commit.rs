//! Commits one holder message to a terminal's readers in a fixed order: the attach
//! hub first, the output waits second.
//!
//! The hub's screen serves `attach.repaint`, `attach.open` first paints,
//! `terminal.screen` and `terminal.output`; the waits keep a screen of their own and
//! answer `terminal.wait_output` from another thread as soon as a chunk resolves them.
//! Feeding the waits first let a client receive a match, ask for a repaint and be
//! answered from a hub that had not taken in the matched bytes yet. Committing the hub
//! first means no reply ever describes a state older than one a completed wait reported.

use std::time::Duration;

use ovis_terminal_core::screen::ReplyBatch;
use ovis_terminal_core::waits::Waits;
use ovis_terminal_holder::protocol::{Exit, Output};
use ovis_terminal_server::streams::TerminalStreams;

use super::super::StageSource;

/// Commits `chunk` to `hub`, then to `waits`, and returns the hub's query replies.
///
/// A chunk the hub refuses reaches no wait. `gap` is the QA seam
/// `--qa-output-commit-gap-ms` (zero otherwise): a pause between the two commits that
/// widens the window a reversed order would expose.
pub(super) fn output(
    (hub, waits): (&TerminalStreams, &Waits),
    chunk: &Output,
    gap: Duration,
) -> Result<ReplyBatch, StageSource> {
    let replies = hub.publish(chunk)?;
    if !gap.is_zero() {
        std::thread::sleep(gap);
    }
    waits.ingest(chunk)?;
    Ok(replies)
}

/// Records the exit on `hub` (its streams get their final bytes and CLOSE), then on
/// `waits`, so a wait that reports the exit never precedes the streams' close.
pub(super) fn exit(
    (hub, waits): (&TerminalStreams, &Waits),
    exit: &Exit,
) -> Result<(), StageSource> {
    hub.finish(exit)?;
    waits.finish(exit)?;
    Ok(())
}

#[cfg(test)]
mod tests;
