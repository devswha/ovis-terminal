//! An in-memory screen and wait hub fed with holder-shaped output, exits and reports.

use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use ovis_terminal_core::input::receipts::InputSeq;
use ovis_terminal_core::screen::{Screen, ScreenBounds, ScreenConfig};
use ovis_terminal_core::waits::{StatusFact, TurnPosition, Waits};
use ovis_terminal_holder::protocol::{
    Exit, Output, ProcessIncarnation, TerminalId, TerminalIncarnation, TerminalTarget,
};
use ovis_terminal_protocol::schema::records::AgentState;
use ovis_terminal_protocol::schema::scalar::{ReportSeq, TurnNumber};
use ovis_terminal_vt::{Geometry, HistoryLimit, OutputOffset, VtConfig};

use super::TestResult;

/// Bound on every wait a test expects to resolve by a feed.
pub const BOUND: Duration = Duration::from_secs(20);

/// A deadline `BOUND` from now.
///
/// # Errors
///
/// Fails when the clock overflows.
pub fn bound() -> TestResult<Instant> {
    Ok(Instant::now().checked_add(BOUND).ok_or("clock overflow")?)
}

/// The holder identity of the fixture process.
pub fn target() -> TerminalTarget {
    TerminalTarget {
        terminal_id: TerminalId::new("terminal:27".into()),
        terminal_incarnation: TerminalIncarnation::new(1),
        process_incarnation: Some(ProcessIncarnation::new(1)),
    }
}

/// A hub over a fresh screen retaining `retained` raw bytes.
///
/// # Errors
///
/// Fails when the screen bounds are rejected.
pub fn hub(retained: usize) -> TestResult<(Waits, Screen)> {
    let bytes = NonZeroUsize::new(retained).ok_or("retention must be positive")?;
    let slice = NonZeroUsize::new(retained / 4).unwrap_or(NonZeroUsize::MIN);
    let bounds = ScreenBounds::DEFAULT
        .with_retained_output(bytes)
        .with_work_budget(slice)
        .with_checkpoint_interval(slice);
    let config = ScreenConfig::new(
        VtConfig::new(Geometry::new(80, 24)?, HistoryLimit::new(100)),
        bounds,
    );
    let screen = Screen::new(target(), config)?;
    Ok((Waits::new(screen.clone()), screen))
}

/// Feeds `bytes` as the holder's next output chunk.
///
/// # Errors
///
/// Fails when the hub refuses the chunk.
pub fn print(waits: &Waits, bytes: &[u8]) -> TestResult {
    let output = Output {
        target: target(),
        offset: waits.cursor()?.get(),
        bytes: bytes.to_vec(),
        hangup: None,
    };
    waits.ingest(&output)?;
    Ok(())
}

/// Reports the exit of the fixture process with `code` after its last byte.
///
/// # Errors
///
/// Fails when the hub refuses the exit.
pub fn exit(waits: &Waits, code: i32) -> TestResult {
    waits.finish(&Exit {
        target: target(),
        code: Some(code),
        signal: None,
        exit_unknown: None,
        io_failure: None,
        final_offset: waits.cursor()?.get(),
    })?;
    Ok(())
}

/// A report of `state` at `seq` in `turn`, after input `input` reached the program.
pub const fn fact(seq: u64, state: AgentState, turn: (u64, u64)) -> StatusFact {
    StatusFact::new(
        ReportSeq::new(seq),
        state,
        TurnPosition::new(TurnNumber::new(turn.0), InputSeq::new(turn.1)),
    )
}

/// An offset.
pub const fn at(offset: u64) -> OutputOffset {
    OutputOffset::new(offset)
}
