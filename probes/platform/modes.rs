//! Terminal-mode round trip: modes a program sets travel through the real
//! terminal and are recovered by the VT engine, then reset the same way.

use std::time::Duration;

use ovis_terminal_vt::{Geometry, HistoryLimit, PasteMode, ScreenBuffer, Vt, VtConfig};
use portable_pty::CommandBuilder;
use serde::Serialize;

use super::error::ProbeError;
use super::session::{Grid, PtySession};

/// Modes the VT engine read from the program's output at one point.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ModeSnapshot {
    /// Bracketed paste (DEC 2004).
    pub paste: PasteMode,
    /// Active screen buffer (DEC 1049).
    pub buffer: ScreenBuffer,
}

/// Modes after the program set them and after it reset them.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ModeRoundTrip {
    /// After `ESC[?2004h ESC[?1049h`.
    pub set: ModeSnapshot,
    /// After `ESC[?1049l ESC[?2004l`.
    pub reset: ModeSnapshot,
}

#[cfg(unix)]
fn program() -> CommandBuilder {
    let mut command = CommandBuilder::new("sh");
    command.args([
        "-c",
        r"printf '\033[?2004h\033[?1049hMODES-SET\n'; read go; printf '\033[?1049l\033[?2004lMODES-RESET\n'; read go",
    ]);
    command
}

#[cfg(windows)]
fn program() -> CommandBuilder {
    let mut command = CommandBuilder::new("powershell.exe");
    command.args([
        "-NoProfile",
        "-Command",
        "$e=[char]27; [Console]::Out.Write(\"$e[?2004h$e[?1049hMODES-SET`n\"); [void][Console]::ReadLine(); [Console]::Out.Write(\"$e[?1049l$e[?2004lMODES-RESET`n\"); [void][Console]::ReadLine()",
    ]);
    command
}

/// Runs a program that sets and resets bracketed paste and the alternate
/// screen, feeding everything it prints through a fresh VT engine.
///
/// # Errors
///
/// Any session error, and [`ProbeError::InvalidGrid`] for a grid the VT engine
/// cannot hold.
pub fn probe_mode_round_trip(grid: Grid, bound: Duration) -> Result<ModeRoundTrip, ProbeError> {
    let geometry = Geometry::new(grid.columns, grid.rows)
        .map_err(|source| ProbeError::InvalidGrid { source })?;
    let mut probe = Probe {
        vt: Vt::new(VtConfig::new(geometry, HistoryLimit::new(100))),
        session: PtySession::spawn(program(), grid)?,
        bound,
    };
    let set = probe.advance(b"MODES-SET")?;
    probe.session.write(b"\r")?;
    let reset = probe.advance(b"MODES-RESET")?;
    probe.session.write(b"\r")?;
    probe.session.finish(bound)?;
    Ok(ModeRoundTrip { set, reset })
}

struct Probe {
    vt: Vt,
    session: PtySession,
    bound: Duration,
}

impl Probe {
    fn advance(&mut self, marker: &[u8]) -> Result<ModeSnapshot, ProbeError> {
        self.vt.feed(&self.session.read_until(marker, self.bound)?);
        let modes = self.vt.modes();
        Ok(ModeSnapshot {
            paste: modes.paste,
            buffer: modes.screen.buffer,
        })
    }
}
