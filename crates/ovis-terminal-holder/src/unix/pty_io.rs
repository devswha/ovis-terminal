//! Non-blocking PTY master I/O that always wakes when the terminal hangs up.
//!
//! A write blocked inside the kernel on a full PTY is not reliably woken when the
//! slave side closes (Linux re-checks the slave's open count before it reaches
//! zero). The master is therefore non-blocking, and the pump and writer wait in
//! `poll(2)`, whose hang-up flag is set before the wakeup. A hang-up (EOF, EIO,
//! POLLHUP) ends a terminal normally; every other failure is returned typed.

use std::fs::File;
use std::io::{self, ErrorKind, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd};
use std::os::unix::net::UnixStream;

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

/// Dropping this endpoint wakes the output pump during shutdown.
pub(super) struct PumpStop {
    _stream: UnixStream,
}

/// Pump-owned endpoint paired with [`PumpStop`].
pub(super) struct PumpWake(UnixStream);

pub(super) fn pump_stop_pair() -> io::Result<(PumpStop, PumpWake)> {
    UnixStream::pair().map(|(stop, wake)| (PumpStop { _stream: stop }, PumpWake(wake)))
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
    wake: &PumpWake,
) -> Result<ReadOutcome, TerminalIoError> {
    loop {
        match file.read(buffer) {
            Ok(0) => return Ok(ReadOutcome::Hangup),
            Ok(read) => return Ok(ReadOutcome::Data(read)),
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                // After a hang-up the next read drains what remains, then reports EIO.
                match wait_for_read(file.as_fd(), wake.0.as_fd())? {
                    PumpReadiness::Pty(Readiness::Ready) => {}
                    PumpReadiness::Pty(Readiness::Hangup) | PumpReadiness::Stopped => {
                        return Ok(ReadOutcome::Hangup);
                    }
                }
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
                    Ok(Readiness::Ready) => {}
                    Ok(Readiness::Hangup) => break,
                    Err(error) => {
                        failure = Some(error);
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

/// What `poll(2)` reported for one descriptor.
#[derive(Debug, Eq, PartialEq)]
enum Readiness {
    Ready,
    Hangup,
}

enum PumpReadiness {
    Pty(Readiness),
    Stopped,
}

fn wait_for_read(
    pty: BorrowedFd<'_>,
    stop: BorrowedFd<'_>,
) -> Result<PumpReadiness, TerminalIoError> {
    let mut entries = [
        libc::pollfd {
            fd: pty.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: stop.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    loop {
        // SAFETY: `entries` is an initialized two-element pollfd array.
        let status = unsafe { libc::poll(entries.as_mut_ptr(), 2, -1) };
        if status >= 0 {
            if entries[0].revents != 0 {
                return classify(entries[0].revents).map(PumpReadiness::Pty);
            }
            if entries[1].revents != 0 {
                classify(entries[1].revents)?;
                return Ok(PumpReadiness::Stopped);
            }
        } else {
            let source = io::Error::last_os_error();
            if source.kind() != ErrorKind::Interrupted {
                return Err(TerminalIoError::Poll { source });
            }
        }
    }
}

/// Waits until `fd` is ready for `events`. POLLHUP is the normal hang-up;
/// POLLNVAL, or POLLERR without POLLHUP, is a typed failure carrying the flags.
fn wait(fd: BorrowedFd<'_>, events: libc::c_short) -> Result<Readiness, TerminalIoError> {
    poll_raw(fd.as_raw_fd(), events)
}

/// Polls a raw descriptor number; a number that is not open reports POLLNVAL.
fn poll_raw(fd: RawFd, events: libc::c_short) -> Result<Readiness, TerminalIoError> {
    let mut entry = libc::pollfd {
        fd,
        events,
        revents: 0,
    };
    loop {
        // SAFETY: `entry` is one live, initialized pollfd and the count is 1.
        let status = unsafe { libc::poll(&raw mut entry, 1, -1) };
        if status >= 0 {
            return classify(entry.revents);
        }
        let source = io::Error::last_os_error();
        if source.kind() != ErrorKind::Interrupted {
            return Err(TerminalIoError::Poll { source });
        }
    }
}

/// POLLHUP is the normal hang-up; POLLNVAL, or POLLERR without POLLHUP, is a
/// typed failure carrying the raw flags.
const fn classify(revents: libc::c_short) -> Result<Readiness, TerminalIoError> {
    let invalid = revents & libc::POLLNVAL != 0;
    let hangup = revents & libc::POLLHUP != 0;
    let error = revents & libc::POLLERR != 0;
    match (invalid, hangup, error) {
        (false, true, _) => Ok(Readiness::Hangup),
        (false, false, false) => Ok(Readiness::Ready),
        (true, _, _) | (false, false, true) => Err(TerminalIoError::PollFlags {
            revents: u16::from_ne_bytes(revents.to_ne_bytes()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use super::{Readiness, classify, poll_raw, pump_stop_pair, read_chunk, write_prefix};
    use crate::protocol::{PtyIoFailure, PtyOperation};

    #[test]
    fn unexpected_read_failure_is_typed_not_a_hangup() {
        // Given: a descriptor whose reads fail with EISDIR, not a PTY hang-up.
        let mut directory = File::open(std::env::temp_dir()).expect("open directory");
        let (_stop, wake) = pump_stop_pair().expect("pump stop pair");

        // When: the pump reads from it.
        let Err(failure) = read_chunk(&mut directory, &mut [0; 16], &wake) else {
            panic!("a failed read must not look like data or EOF");
        };

        // Then: the failure keeps its kind, source and OS error.
        assert_eq!(failure.kind(), "pty_read_failed");
        assert!(std::error::Error::source(&failure).is_some());
        let wire = PtyIoFailure {
            operation: PtyOperation::Read,
            os_error: Some(libc::EISDIR),
            poll_revents: None,
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

    #[test]
    fn poll_flags_classify_hangup_readiness_and_typed_failures() {
        // Given / When / Then: POLLHUP (with or without POLLERR) is a hang-up,
        // POLLERR without POLLHUP and POLLNVAL are typed failures.
        assert_eq!(classify(libc::POLLHUP).ok(), Some(Readiness::Hangup));
        assert_eq!(
            classify(libc::POLLHUP | libc::POLLERR).ok(),
            Some(Readiness::Hangup)
        );
        assert_eq!(classify(libc::POLLOUT).ok(), Some(Readiness::Ready));
        for flags in [libc::POLLOUT | libc::POLLERR, libc::POLLNVAL] {
            let failure = classify(flags).expect_err("error flags are a failure");
            assert_eq!(failure.kind(), "pty_poll_flagged");
            let wire = failure.io_failure().expect("output-side failure");
            let expected = u16::from_ne_bytes(flags.to_ne_bytes());
            assert_eq!(
                (wire.operation, wire.poll_revents),
                (PtyOperation::Poll, Some(expected))
            );
        }
    }

    #[test]
    fn kernel_poll_error_flags_are_a_typed_failure_not_a_hangup() {
        // Given: a descriptor number that is never open; Linux and macOS both
        // report POLLNVAL for it.
        let never_open = libc::c_int::MAX / 2;

        // When: the holder waits on it.
        let Err(failure) = poll_raw(never_open, libc::POLLOUT) else {
            panic!("POLLNVAL must not be erased into readiness or a hang-up");
        };

        // Then: the kernel's flags arrive as the typed poll failure.
        let pollnval = u16::from_ne_bytes(libc::POLLNVAL.to_ne_bytes());
        let wire = failure.io_failure().expect("output-side failure");
        assert_eq!(wire.operation, PtyOperation::Poll);
        assert_eq!(
            wire.poll_revents.map(|flags| flags & pollnval),
            Some(pollnval)
        );
    }
}
