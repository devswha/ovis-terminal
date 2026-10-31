//! Read/write boundary tests for the non-blocking PTY master.

use std::fs::File;
use std::io::{self, ErrorKind, PipeReader, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};

use super::{ReadOutcome, Readiness, classify, poll_raw, pump_stop_pair, read_chunk, write_prefix};
use crate::protocol::{PtyIoFailure, PtyOperation};

/// A real pipe whose first read finds nothing, as when the final bytes reach
/// the master only after the pump's read and before its `poll`.
struct LateFinalBytes {
    pipe: PipeReader,
    first_read_done: bool,
}

impl Read for LateFinalBytes {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.first_read_done {
            return self.pipe.read(buffer);
        }
        self.first_read_done = true;
        Err(ErrorKind::WouldBlock.into())
    }
}

impl AsFd for LateFinalBytes {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.pipe.as_fd()
    }
}

#[test]
fn hangup_readiness_with_queued_final_bytes_returns_them_before_hangup() {
    // Given: final bytes are queued and the writing side is closed, so the
    // kernel's poll reports POLLIN and POLLHUP together.
    let (pipe, mut writer) = io::pipe().expect("pipe");
    writer.write_all(b"final").expect("queue final bytes");
    drop(writer);
    let mut reader = LateFinalBytes {
        pipe,
        first_read_done: false,
    };
    let (_stop, wake) = pump_stop_pair().expect("pump stop pair");
    let mut buffer = [0; 16];

    // When: the pump reads after its first read found nothing.
    let first = read_chunk(&mut reader, &mut buffer, &wake).expect("read");
    let ReadOutcome::Data(count) = first else {
        panic!("queued final bytes were dropped as a hang-up");
    };
    let bytes = buffer.get(..count).expect("read bounds").to_vec();
    let second = read_chunk(&mut reader, &mut buffer, &wake).expect("read");

    // Then: every queued byte arrives, and only then does output end.
    assert_eq!(bytes, b"final");
    assert!(matches!(second, ReadOutcome::Hangup));
}

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
