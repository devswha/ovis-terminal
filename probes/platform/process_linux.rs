//! Linux process evidence from `/proc`.

use std::fs;

use ovis_terminal_holder::protocol::ProcessStartToken;

use super::{Pid, ProbeError, ProcessObservation, ProcessState};

pub(super) fn observe(pid: Pid) -> Result<ProcessObservation, ProbeError> {
    let stat = read(pid, &format!("/proc/{pid}/stat"))?;
    // The command name may contain spaces and parentheses, so fields are
    // counted from the last ')'; the first field after it is field 3 (state)
    // and the start time is field 22.
    let after_name = stat
        .rfind(')')
        .and_then(|close| stat.get(close.saturating_add(1)..))
        .ok_or(ProbeError::ProcessRecordMalformed { pid, field: "comm" })?;
    let mut fields = after_name.split_whitespace();
    let state = fields.next().ok_or(ProbeError::ProcessRecordMalformed {
        pid,
        field: "state",
    })?;
    let start_ticks = fields
        .nth(18)
        .filter(|ticks| ticks.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or(ProbeError::ProcessRecordMalformed {
            pid,
            field: "starttime",
        })?;
    let boot_id = read(pid, "/proc/sys/kernel/random/boot_id")?;
    Ok(ProcessObservation {
        pid,
        start_token: ProcessStartToken::new(format!("linux:{}:{start_ticks}", boot_id.trim())),
        state: if state == "Z" {
            ProcessState::Zombie
        } else {
            ProcessState::Live
        },
    })
}

pub(super) fn resident_kib(pid: Pid) -> Result<u64, ProbeError> {
    let status = read(pid, &format!("/proc/{pid}/status"))?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|value| value.trim().strip_suffix("kB"))
        .and_then(|kib| kib.trim().parse().ok())
        .ok_or(ProbeError::ProcessRecordMalformed {
            pid,
            field: "VmRSS",
        })
}

fn read(pid: Pid, path: &str) -> Result<String, ProbeError> {
    fs::read_to_string(path).map_err(|source| ProbeError::ProcessRecordUnreadable { pid, source })
}
