//! Non-blocking PTY master I/O that always wakes when the terminal hangs up.
//!
//! A write blocked inside the kernel on a full PTY is not reliably woken when the
//! slave side closes (Linux re-checks the slave's open count before it reaches
//! zero). The master is therefore non-blocking, and the pump and writer wait in
//! `poll(2)`, whose hang-up flag is set before the wakeup, so a dead terminal
//! always releases its writer with the bytes it actually wrote.

use std::fs::File;
use std::io::{self, ErrorKind, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};

/// Outcome of one bounded read from the master.
pub(super) enum ReadOutcome {
    Data(usize),
    End,
}

/// Puts the master's open file description (shared by every duplicate) in
/// non-blocking mode.
pub(super) fn set_nonblocking(fd: BorrowedFd<'_>) -> io::Result<()> {
    // SAFETY: F_GETFL on a descriptor borrowed for this call reads no memory.
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: F_SETFL takes an integer flag set and reads no memory.
    let status = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if status == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Reads the next chunk, waiting while the master is empty; `End` at EOF, hang-up
/// or any read failure.
pub(super) fn read_chunk(file: &mut File, buffer: &mut [u8]) -> ReadOutcome {
    loop {
        match file.read(buffer) {
            Ok(0) => return ReadOutcome::End,
            Ok(read) => return ReadOutcome::Data(read),
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if wait(file.as_fd(), libc::POLLIN).is_err() {
                    return ReadOutcome::End;
                }
            }
            // EIO is how a PTY master reports that the slave side closed.
            Err(_) => return ReadOutcome::End,
        }
    }
}

/// Writes as much of `bytes` as the PTY accepts and returns the actual count; a
/// hang-up or write failure ends the entry with the prefix already written.
pub(super) fn write_prefix(file: &mut File, bytes: &[u8]) -> usize {
    let mut written = 0_usize;
    while let Some(rest) = bytes.get(written..).filter(|rest| !rest.is_empty()) {
        match file.write(rest) {
            Ok(0) => break,
            Ok(count) => written = written.saturating_add(count),
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if !matches!(wait(file.as_fd(), libc::POLLOUT), Ok(true)) {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    written
}

/// Waits until `fd` is ready for `events`; `Ok(false)` when it hung up instead.
fn wait(fd: BorrowedFd<'_>, events: libc::c_short) -> io::Result<bool> {
    let mut entry = libc::pollfd {
        fd: fd.as_raw_fd(),
        events,
        revents: 0,
    };
    loop {
        // SAFETY: `entry` is one live, initialized pollfd and the count is 1.
        let status = unsafe { libc::poll(&raw mut entry, 1, -1) };
        if status >= 0 {
            return Ok(entry.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) == 0);
        }
        let error = io::Error::last_os_error();
        if error.kind() != ErrorKind::Interrupted {
            return Err(error);
        }
    }
}
