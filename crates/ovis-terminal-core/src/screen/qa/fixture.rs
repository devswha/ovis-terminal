//! Shared builders for the screen integration tests.

use std::num::{NonZeroU32, NonZeroUsize};

use ovis_terminal_core::screen::{Ingested, Needle, Screen, ScreenBounds, ScreenConfig};
use ovis_terminal_holder::protocol::{Output, TerminalId, TerminalIncarnation, TerminalTarget};
use ovis_terminal_vt::{ContentRevision, Geometry, HistoryLimit, OutputOffset, VtConfig};

/// Result of a fallible fixture step.
pub type Fallible<T> = Result<T, Box<dyn std::error::Error>>;
/// Outcome of an assertion-bearing case.
pub type TestResult = Fallible<()>;

/// The holder identity of a fixture terminal.
pub fn target(name: &str) -> TerminalTarget {
    TerminalTarget {
        terminal_id: TerminalId::new(name.to_owned()),
        terminal_incarnation: TerminalIncarnation::new(1),
        process_incarnation: None,
    }
}

/// One holder output chunk.
pub fn output(target: &TerminalTarget, offset: u64, bytes: &[u8]) -> Output {
    Output {
        target: target.clone(),
        offset,
        bytes: bytes.to_vec(),
        hangup: None,
    }
}

/// `value` as a non-zero bound; fixtures pass positive literals.
pub const fn nonzero(value: usize) -> NonZeroUsize {
    NonZeroUsize::MIN.saturating_add(value.saturating_sub(1))
}

/// `value` as a non-zero limit; fixtures pass positive literals.
pub const fn limit(value: u32) -> NonZeroU32 {
    NonZeroU32::MIN.saturating_add(value.saturating_sub(1))
}

/// Emulator configuration with a 1000-line history.
///
/// # Errors
///
/// Returns the underlying construction or ingestion failure.
pub fn vt_config(columns: u16, rows: u16) -> Fallible<VtConfig> {
    Ok(VtConfig::new(
        Geometry::new(columns, rows)?,
        HistoryLimit::new(1000),
    ))
}

/// A screen for a fixture terminal with a 40x5 grid.
///
/// # Errors
///
/// Returns the underlying construction or ingestion failure.
pub fn screen(name: &str, bounds: ScreenBounds) -> Fallible<Screen> {
    Ok(Screen::new(
        target(name),
        ScreenConfig::new(vt_config(40, 5)?, bounds),
    )?)
}

/// Feeds `bytes` as one holder chunk at the screen's live offset.
///
/// # Errors
///
/// Returns the underlying construction or ingestion failure.
pub fn feed(screen: &Screen, bytes: &[u8]) -> Fallible<Ingested> {
    let at = screen.live()?.offset.get();
    Ok(screen.ingest(&output(screen.target(), at, bytes))?)
}

/// `line N` rows separated by CR LF.
pub fn numbered_lines(count: usize) -> Vec<u8> {
    (0..count)
        .flat_map(|line| format!("line {line}\r\n").into_bytes())
        .collect()
}

/// A content revision decoded from its decimal wire form.
///
/// # Errors
///
/// Returns the underlying construction or ingestion failure.
pub fn revision(value: u64) -> Fallible<ContentRevision> {
    Ok(serde_json::from_value(serde_json::Value::String(
        value.to_string(),
    ))?)
}

/// A raw output offset.
pub const fn offset(value: u64) -> OutputOffset {
    OutputOffset::new(value)
}

/// A non-empty search needle.
///
/// # Errors
///
/// Returns the underlying construction or ingestion failure.
pub fn needle(text: &str) -> Fallible<Needle> {
    Ok(Needle::new(text.to_owned())?)
}
