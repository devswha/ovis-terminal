//! macOS process records: `proc_pidinfo` start time, process group and controlling tty.

use std::ffi::{CStr, c_int};
use std::io;
use std::mem::{MaybeUninit, size_of, size_of_val};
use std::path::PathBuf;

use super::table::{Membership, ProbeError, ProcessEntry, TerminalMembership};
use super::{ProcessId, StartTokenError};
use crate::protocol::ProcessStartToken;

/// `SZOMB` from `<sys/proc.h>`: exited, not yet reaped.
const ZOMBIE: u32 = 5;
/// `NODEV`: the process has no controlling terminal.
const NO_DEVICE: u32 = u32::MAX;
/// `PROC_TTY_ONLY` from `<sys/proc_info.h>`: list processes by controlling terminal.
const TTY_ONLY: u32 = 3;

fn raw_pid(pid: ProcessId) -> Result<c_int, ProbeError> {
    c_int::try_from(pid.get()).map_err(|_| ProbeError::Gone { pid: pid.get() })
}

fn os_error(pid: ProcessId) -> ProbeError {
    let source = io::Error::last_os_error();
    match source.raw_os_error() {
        Some(libc::ESRCH) => ProbeError::Gone { pid: pid.get() },
        Some(libc::EPERM | libc::EACCES) => ProbeError::Permission { source },
        _ => ProbeError::Unreadable { source },
    }
}

/// Reads one fixed-size `proc_pidinfo` flavor into `T`.
fn pidinfo<T>(pid: ProcessId, flavor: c_int) -> Result<T, ProbeError> {
    let size = c_int::try_from(size_of::<T>()).map_err(|_| ProbeError::Malformed)?;
    let mut info = MaybeUninit::<T>::zeroed();
    // SAFETY: the buffer is a live, writable `T` of exactly `size` bytes, and the kernel
    // writes at most `size` bytes into it.
    let written =
        unsafe { libc::proc_pidinfo(raw_pid(pid)?, flavor, 0, info.as_mut_ptr().cast(), size) };
    if written == size {
        // SAFETY: the kernel filled all `size` bytes of the zero-initialised plain-data struct.
        Ok(unsafe { info.assume_init() })
    } else if written <= 0 {
        Err(os_error(pid))
    } else {
        Err(ProbeError::Malformed)
    }
}

fn token(info: &libc::proc_bsdinfo) -> ProcessStartToken {
    ProcessStartToken::new(format!(
        "darwin:{}.{:06}",
        info.pbi_start_tvsec, info.pbi_start_tvusec
    ))
}

pub(super) fn start_token(pid: ProcessId) -> Result<ProcessStartToken, StartTokenError> {
    let info: libc::proc_bsdinfo = pidinfo(pid, libc::PROC_PIDTBSDINFO)?;
    Ok(token(&info))
}

pub(super) fn entry(pid: ProcessId) -> Result<ProcessEntry, ProbeError> {
    let info: libc::proc_bsdinfo = pidinfo(pid, libc::PROC_PIDTBSDINFO)?;
    let terminal = if info.e_tdev == NO_DEVICE {
        Membership::Detached
    } else {
        Membership::Controlling(TerminalMembership {
            group: info.pbi_pgid,
            tty: u64::from(info.e_tdev),
            foreground_group: info.e_tpgid,
        })
    };
    Ok(ProcessEntry {
        pid,
        start_token: token(&info),
        running: info.pbi_status != ZOMBIE,
        terminal,
    })
}

/// PIDs whose controlling terminal is `tty`, from the kernel's own per-tty index, so
/// processes of other users and terminals are never read.
pub(super) fn terminal_pids(tty: u64) -> Result<Vec<ProcessId>, ProbeError> {
    let device = u32::try_from(tty).map_err(|_| ProbeError::Malformed)?;
    let unreadable = || ProbeError::Unreadable {
        source: io::Error::last_os_error(),
    };
    // SAFETY: a null buffer of size zero asks only for the byte size of the list.
    let needed = unsafe { libc::proc_listpids(TTY_ONLY, device, std::ptr::null_mut(), 0) };
    let slots = usize::try_from(needed)
        .map_err(|_| unreadable())?
        .checked_div(size_of::<c_int>())
        .ok_or(ProbeError::Malformed)?
        .saturating_add(64);
    let mut buffer: Vec<c_int> = vec![0; slots];
    let bytes = c_int::try_from(slots.saturating_mul(size_of::<c_int>()))
        .map_err(|_| ProbeError::Malformed)?;
    // SAFETY: the buffer holds `bytes` writable bytes of `c_int` slots.
    let written =
        unsafe { libc::proc_listpids(TTY_ONLY, device, buffer.as_mut_ptr().cast(), bytes) };
    let listed = usize::try_from(written)
        .map_err(|_| unreadable())?
        .checked_div(size_of::<c_int>())
        .ok_or(ProbeError::Malformed)?;
    buffer.truncate(listed);
    Ok(buffer
        .into_iter()
        .filter_map(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid > 0)
        .map(ProcessId::new)
        .collect())
}

pub(super) fn cwd(pid: ProcessId) -> Result<PathBuf, ProbeError> {
    let info: libc::proc_vnodepathinfo = pidinfo(pid, libc::PROC_PIDVNODEPATHINFO)?;
    let raw = &info.pvi_cdir.vip_path;
    // SAFETY: `raw` is a plain byte array inside `info`, borrowed for this scope only.
    let bytes = unsafe {
        std::slice::from_raw_parts(std::ptr::from_ref(raw).cast::<u8>(), size_of_val(raw))
    };
    let path = CStr::from_bytes_until_nul(bytes).map_err(|_| ProbeError::Malformed)?;
    Ok(PathBuf::from(
        path.to_str().map_err(|_| ProbeError::Malformed)?,
    ))
}

fn sysctl(mib: &mut [c_int], buffer: &mut [u8]) -> Result<usize, ProbeError> {
    let depth = libc::c_uint::try_from(mib.len()).map_err(|_| ProbeError::Malformed)?;
    let mut size = buffer.len();
    // SAFETY: `mib` names `depth` integers and `buffer` holds `size` writable bytes; no
    // new value is written.
    let status = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            depth,
            buffer.as_mut_ptr().cast(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if status == 0 {
        Ok(size)
    } else {
        Err(ProbeError::Unreadable {
            source: io::Error::last_os_error(),
        })
    }
}

/// `KERN_PROCARGS2` reports `EINVAL`, not `ESRCH`, for a PID with no process, so a
/// failure is attributed to a vanished process only when `proc_pidinfo` confirms it.
fn gone_or(pid: ProcessId, error: ProbeError) -> ProbeError {
    match pidinfo::<libc::proc_bsdinfo>(pid, libc::PROC_PIDTBSDINFO) {
        Err(gone @ ProbeError::Gone { .. }) => gone,
        Ok(_) | Err(_) => error,
    }
}

/// `KERN_PROCARGS2`: argc, the executable path, padding, then argc strings.
pub(super) fn argv(pid: ProcessId) -> Result<Vec<String>, ProbeError> {
    let mut limit = [0_u8; size_of::<c_int>()];
    sysctl(&mut [libc::CTL_KERN, libc::KERN_ARGMAX], &mut limit)?;
    let limit = usize::try_from(c_int::from_ne_bytes(limit)).map_err(|_| ProbeError::Malformed)?;
    let mut buffer = vec![0_u8; limit];
    let used = sysctl(
        &mut [libc::CTL_KERN, libc::KERN_PROCARGS2, raw_pid(pid)?],
        &mut buffer,
    )
    .map_err(|error| gone_or(pid, error))?;
    let record = buffer.get(..used).ok_or(ProbeError::Malformed)?;
    let (count, rest) = record
        .split_first_chunk::<{ size_of::<c_int>() }>()
        .ok_or(ProbeError::Malformed)?;
    let count = usize::try_from(c_int::from_ne_bytes(*count)).map_err(|_| ProbeError::Malformed)?;
    let after_path = rest
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(ProbeError::Malformed)?;
    rest.get(after_path..)
        .ok_or(ProbeError::Malformed)?
        .split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .take(count)
        .map(|arg| String::from_utf8(arg.to_vec()).map_err(|_| ProbeError::Malformed))
        .collect()
}
