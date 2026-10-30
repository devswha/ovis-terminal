//! macOS process evidence from `ps`, which reads the kernel process table.

use std::io;
use std::process::Command;

use ovis_terminal_holder::protocol::ProcessStartToken;

use super::{Pid, ProbeError, ProcessObservation, ProcessState};

pub(super) fn observe(pid: Pid) -> Result<ProcessObservation, ProbeError> {
    let record = ps(pid, "stat=,lstart=")?;
    let (state, started) = record.trim().split_once(char::is_whitespace).ok_or(
        ProbeError::ProcessRecordMalformed {
            pid,
            field: "lstart",
        },
    )?;
    Ok(ProcessObservation {
        pid,
        start_token: ProcessStartToken::new(format!("macos-lstart:{}", started.trim())),
        state: if state.starts_with('Z') {
            ProcessState::Zombie
        } else {
            ProcessState::Live
        },
    })
}

pub(super) fn resident_kib(pid: Pid) -> Result<u64, ProbeError> {
    ps(pid, "rss=")?
        .trim()
        .parse()
        .map_err(|_unparsable| ProbeError::ProcessRecordMalformed { pid, field: "rss" })
}

fn ps(pid: Pid, format: &str) -> Result<String, ProbeError> {
    let output = Command::new("ps")
        .args(["-o", format, "-p", &pid.to_string()])
        .output()
        .map_err(|source| ProbeError::ProcessRecordUnreadable { pid, source })?;
    if !output.status.success() {
        return Err(ProbeError::ProcessRecordUnreadable {
            pid,
            source: io::Error::new(io::ErrorKind::NotFound, "ps lists no such process"),
        });
    }
    String::from_utf8(output.stdout).map_err(|_not_utf8| ProbeError::ProcessRecordMalformed {
        pid,
        field: "ps output",
    })
}
