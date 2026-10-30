//! Non-blocking PTY master I/O that always wakes when the terminal hangs up.
//!
//! A write blocked inside the kernel on a full PTY is not reliably woken when the
//! slave side closes (Linux re-checks the slave's open count before it reaches
//! zero). The master is therefore non-blocking, and the pump and writer wait in
//! `poll(2)`, whose hang-up flag is set before the wakeup. A hang-up (EOF, EIO,
//! POLLHUP) ends a terminal normally; every other failure is returned typed.

use std::fs::File;
use std::io::{self, ErrorKind, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};

use super::io_error::TerminalIoError;

/// Outcome of one bounded read from the master.
pub(super) enum ReadOutcome {
    Data(usize),
    Hangup,
}

/// Bytes the PTY accepted for one entry, and the failure that cut it short.
pub(super) struct WriteOutcome {
    pub(super) written: usize,
    pub(super) failure: Option<TerminalIoError>,
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

/// EIO is how a PTY master reports that the slave side closed.
fn is_hangup(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::EIO)
}

/// Reads the next chunk, waiting while the master is empty.
pub(super) fn read_chunk(
    file: &mut File,
    buffer: &mut [u8],
) -> Result<ReadOutcome, TerminalIoError> {
    loop {
        match file.read(buffer) {
            Ok(0) => return Ok(ReadOutcome::Hangup),
            Ok(read) => return Ok(ReadOutcome::Data(read)),
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                // After a hang-up the next read drains what remains, then reports EIO.
                wait(file.as_fd(), libc::POLLIN)
                    .map_err(|source| TerminalIoError::Poll { source })?;
            }
            Err(error) if is_hangup(&error) => return Ok(ReadOutcome::Hangup),
            Err(source) => return Err(TerminalIoError::Read { source }),
        }
    }
}

/// Writes as much of `bytes` as the PTY accepts and returns the actual count; a
/// hang-up ends the entry quietly, any other failure is returned with it.
pub(super) fn write_prefix(file: &mut File, bytes: &[u8]) -> WriteOutcome {
    let mut written = 0_usize;
    let mut failure = None;
    while let Some(rest) = bytes.get(written..).filter(|rest| !rest.is_empty()) {
        match file.write(rest) {
            Ok(0) => break,
            Ok(count) => written = written.saturating_add(count),
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                match wait(file.as_fd(), libc::POLLOUT) {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(source) => {
                        failure = Some(TerminalIoError::Poll { source });
                        break;
                    }
                }
            }
            Err(error) if is_hangup(&error) => break,
            Err(source) => {
                failure = Some(TerminalIoError::Write { source });
                break;
            }
        }
    }
    WriteOutcome { written, failure }
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

#[cfg(test)]
mod tests {
    use std::fs::File;

    use super::{read_chunk, write_prefix};
    use crate::protocol::{PtyIoFailure, PtyOperation};

    #[test]
    fn unexpected_read_failure_is_typed_not_a_hangup() {
        // Given: a descriptor whose reads fail with EISDIR, not a PTY hang-up.
        let mut directory = File::open(std::env::temp_dir()).expect("open directory");

        // When: the pump reads from it.
        let Err(failure) = read_chunk(&mut directory, &mut [0; 16]) else {
            panic!("a failed read must not look like data or EOF");
        };

        // Then: the failure keeps its kind, source and OS error.
        assert_eq!(failure.kind(), "pty_read_failed");
        assert!(std::error::Error::source(&failure).is_some());
        let wire = PtyIoFailure {
            operation: PtyOperation::Read,
            os_error: Some(libc::EISDIR),
        };
        assert_eq!(failure.io_failure(), Some(wire));
        assert_eq!(failure.exit_unknown(), None);
    }

    #[test]
    fn unexpected_write_failure_is_returned_with_the_written_prefix() {
        // Given: a read-only descriptor, so writes fail with EBADF.
        let path = std::env::temp_dir().join(format!("ovis-t6-ro-{}", std::process::id()));
        std::fs::write(&path, b"").expect("create file");
        let mut read_only = File::open(&path).expect("open read-only");

        // When: the writer writes an entry to it.
        let outcome = write_prefix(&mut read_only, b"input");

        // Then: nothing was written and the failure is typed with its OS error.
        assert_eq!(outcome.written, 0);
        let failure = outcome.failure.expect("write failure is reported");
        assert_eq!(failure.kind(), "pty_write_failed");
        let wire = failure.io_failure().expect("output-side failure");
        assert_eq!(
            (wire.operation, wire.os_error),
            (PtyOperation::Write, Some(libc::EBADF))
        );
        std::fs::remove_file(&path).expect("remove file");
    }
}
