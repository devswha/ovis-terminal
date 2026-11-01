//! An independent VT image of a terminal's raw output, to check the daemon's screen.

use base64::Engine as _;
use ovis_terminal_vt::{Geometry, HistoryLimit, Vt, VtConfig};
use serde_json::{Value, json};

use super::super::support::TestResult;

/// The visible lines and the `{"row", "col", "visible"}` cursor of a fresh 80x24 VT fed the
/// base64 `raw` output in one piece, whatever chunks the daemon received it in.
pub(super) fn replay(raw: &str) -> TestResult<(Vec<String>, Value)> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(raw)?;
    let mut vt = Vt::new(VtConfig::new(
        Geometry::new(80, 24)?,
        HistoryLimit::new(10_000),
    ));
    vt.feed(&bytes);
    let lines = vt
        .screen()
        .lines
        .into_iter()
        .map(|line| line.text)
        .collect();
    let cursor = vt.cursor();
    Ok((
        lines,
        json!({"row": cursor.row, "col": cursor.column, "visible": cursor.visible}),
    ))
}
