//! Measured footprints: resident memory and CPU time of the runtime and its programs,
//! bytes the run keeps on disk.

use std::path::Path;
use std::time::Duration;

use ovis_terminal_probes::platform::{ByteCount, Pid, resident_memory};

use super::error::Terminal50Error;

/// Resident bytes of this process plus every live program in `pids`.
pub(super) fn rss(pids: &[Pid]) -> Result<u64, Terminal50Error> {
    let mut total = resident_memory(Pid::new(std::process::id()))?.get();
    for pid in pids {
        total = total.saturating_add(resident_memory(*pid).map_or(0, ByteCount::get));
    }
    Ok(total)
}

/// Bytes of every file below `root`, from file metadata.
pub(super) fn disk_bytes(root: &Path) -> Result<u64, Terminal50Error> {
    let mut total = 0_u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    Ok(total)
}

fn cpu_error(pid: u32) -> impl FnOnce(String) -> Terminal50Error {
    move |reason| Terminal50Error::Cpu {
        pid,
        source: reason.into(),
    }
}

/// CPU time every thread of `pid` has used: utime plus stime of `/proc/<pid>/stat`.
#[cfg(target_os = "linux")]
pub(super) fn cpu_time(pid: u32) -> Result<Duration, Terminal50Error> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|error| cpu_error(pid)(error.to_string()))?;
    let ticks = clock_ticks().ok_or_else(|| cpu_error(pid)("getconf CLK_TCK failed".to_owned()))?;
    stat_cpu(&stat, ticks).ok_or_else(|| cpu_error(pid)(format!("unparsable stat {stat:?}")))
}

#[cfg(target_os = "linux")]
fn clock_ticks() -> Option<u64> {
    let output = std::process::Command::new("getconf")
        .arg("CLK_TCK")
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()
        .filter(|ticks| *ticks > 0)
}

/// utime + stime (fields 14 and 15, counted after the parenthesised command) in seconds.
#[cfg(any(target_os = "linux", test))]
pub(super) fn stat_cpu(stat: &str, ticks_per_second: u64) -> Option<Duration> {
    let (_, rest) = stat.rsplit_once(')')?;
    let mut fields = rest.split_whitespace().skip(11);
    let user: u64 = fields.next()?.parse().ok()?;
    let system: u64 = fields.next()?.parse().ok()?;
    let ticks = user.checked_add(system)?;
    let whole = ticks.checked_div(ticks_per_second)?;
    let rest = ticks.checked_rem(ticks_per_second)?;
    let nanos = rest
        .checked_mul(1_000_000_000)?
        .checked_div(ticks_per_second)?;
    Some(Duration::from_secs(whole).saturating_add(Duration::from_nanos(nanos)))
}

/// CPU time `pid` has used, from `ps -o time=` (`[[hh:]mm:]ss.cc`).
#[cfg(not(target_os = "linux"))]
pub(super) fn cpu_time(pid: u32) -> Result<Duration, Terminal50Error> {
    let output = std::process::Command::new("ps")
        .args(["-o", "time=", "-p", &pid.to_string()])
        .output()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() {
        return Err(cpu_error(pid)(format!("ps lists no process {pid}")));
    }
    parse_clock(&text).ok_or_else(|| cpu_error(pid)(format!("unparsable time {text:?}")))
}

/// Parses `[[hh:]mm:]ss[.fraction]` as a duration.
#[cfg(any(not(target_os = "linux"), test))]
pub(super) fn parse_clock(text: &str) -> Option<Duration> {
    let mut seconds = 0.0_f64;
    for field in text.split(':') {
        let value: f64 = field.trim().parse().ok()?;
        if !value.is_finite() || value < 0.0 {
            return None;
        }
        seconds = seconds.mul_add(60.0, value);
    }
    Duration::try_from_secs_f64(seconds).ok()
}

/// CPU time of this process and the summed CPU time of the programs in `pids`.
pub(super) fn cpu(pids: &[Pid]) -> Result<(Duration, Duration), Terminal50Error> {
    let runtime = cpu_time(std::process::id())?;
    let mut programs = Duration::ZERO;
    for pid in pids {
        programs = programs.saturating_add(cpu_time(pid.get())?);
    }
    Ok((runtime, programs))
}
