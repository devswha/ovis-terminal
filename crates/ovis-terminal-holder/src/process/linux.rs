//! Linux process records: `/proc/<pid>/stat` start ticks bound to the boot ID.

use std::io;
use std::path::PathBuf;

use super::ProcessId;
use super::table::{Membership, ProbeError, ProcessEntry, TerminalMembership};
use crate::protocol::ProcessStartToken;

/// Fields after the command name, which may itself contain spaces or parentheses.
struct Stat {
    state: String,
    group: u32,
    tty: u64,
    foreground: i64,
    start_ticks: u64,
}

fn read_stat(pid: ProcessId) -> Result<Stat, ProbeError> {
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid.get()))
        .map_err(|source| ProbeError::from_io(pid, source))?;
    let (_, fields) = stat.rsplit_once(')').ok_or(ProbeError::Malformed)?;
    // After the name: state(0) ppid(1) pgrp(2) session(3) tty_nr(4) tpgid(5) ... starttime(19).
    let fields: Vec<&str> = fields.split_whitespace().collect();
    let field = |index: usize| fields.get(index).copied().ok_or(ProbeError::Malformed);
    let number = |index: usize| {
        field(index)?
            .parse::<i64>()
            .map_err(|_| ProbeError::Malformed)
    };
    Ok(Stat {
        state: field(0)?.to_owned(),
        group: u32::try_from(number(2)?).map_err(|_| ProbeError::Malformed)?,
        tty: u64::try_from(number(4)?).map_err(|_| ProbeError::Malformed)?,
        foreground: number(5)?,
        start_ticks: field(19)?.parse().map_err(|_| ProbeError::Malformed)?,
    })
}

fn token(start_ticks: u64) -> Result<ProcessStartToken, ProbeError> {
    let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_err(|source| ProbeError::Unreadable { source })?;
    Ok(ProcessStartToken::new(format!(
        "linux:{}:{start_ticks}",
        boot_id.trim()
    )))
}

/// Reads the record of `pid`.
///
/// # Errors
///
/// Returns [`ProbeError::Gone`] when no process has the PID, or a typed read failure.
pub fn read_entry(pid: ProcessId) -> Result<ProcessEntry, ProbeError> {
    let stat = read_stat(pid)?;
    let running = match stat.state.as_str() {
        "Z" | "X" | "x" => false,
        "R" | "S" | "D" | "T" | "t" | "K" | "W" | "P" | "I" => true,
        _ => return Err(ProbeError::Malformed),
    };
    let terminal = match u32::try_from(stat.foreground) {
        Ok(foreground_group) if stat.tty != 0 => Membership::Controlling(TerminalMembership {
            group: stat.group,
            tty: stat.tty,
            foreground_group,
        }),
        Ok(_) | Err(_) => Membership::Detached,
    };
    Ok(ProcessEntry {
        pid,
        start_token: token(stat.start_ticks)?,
        running,
        terminal,
    })
}

/// Lists the PIDs that may have `tty` as their controlling terminal; callers re-check
/// each record's membership.
///
/// # Errors
///
/// Returns [`ProbeError::Unsupported`] where the table cannot be enumerated.
pub fn list_terminal_pids(_: u64) -> Result<Vec<ProcessId>, ProbeError> {
    let mut pids = Vec::new();
    for entry in std::fs::read_dir("/proc").map_err(|source| ProbeError::Unreadable { source })? {
        let entry = entry.map_err(|source| ProbeError::Unreadable { source })?;
        if let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok())
        {
            pids.push(ProcessId::new(pid));
        }
    }
    Ok(pids)
}

/// Reads the current argument vector of `pid`.
///
/// # Errors
///
/// Returns [`ProbeError::Malformed`] for a non-UTF-8 argument, or a typed read failure.
pub fn read_argv(pid: ProcessId) -> Result<Vec<String>, ProbeError> {
    let bytes = std::fs::read(format!("/proc/{}/cmdline", pid.get()))
        .map_err(|source| ProbeError::from_io(pid, source))?;
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    bytes
        .strip_suffix(&[0])
        .ok_or(ProbeError::Malformed)?
        .split(|byte| *byte == 0)
        .map(|arg| String::from_utf8(arg.to_vec()).map_err(|_| ProbeError::Malformed))
        .collect()
}

/// Reads the current working directory of `pid`.
///
/// # Errors
///
/// Returns a typed read failure, or [`ProbeError::Unsupported`] where unavailable.
pub fn read_cwd(pid: ProcessId) -> Result<PathBuf, ProbeError> {
    std::fs::read_link(format!("/proc/{}/cwd", pid.get()))
        .map_err(|source| ProbeError::from_io(pid, source))
}

impl ProbeError {
    fn from_io(pid: ProcessId, source: io::Error) -> Self {
        match source.kind() {
            io::ErrorKind::NotFound => Self::Gone { pid: pid.get() },
            io::ErrorKind::PermissionDenied => Self::Permission { source },
            _ => Self::Unreadable { source },
        }
    }
}
