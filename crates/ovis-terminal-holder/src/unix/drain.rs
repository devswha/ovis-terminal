//! Exit-time drain barrier between a terminal's waiter and output pump.
//!
//! macOS discards unread PTY output when the last slave reference closes, so the
//! holder keeps its slave open until the child has exited and the master holds
//! no unread bytes; only then does it close the slave and let the pump reach EOF.
//! The pump bumps a read counter after every chunk, so the waiter re-checks the
//! pending byte count exactly when the pump makes progress.

use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::{Condvar, Mutex, PoisonError};

#[derive(Debug, Default)]
struct Progress {
    reads: u64,
    finished: bool,
}

/// Read progress published by a pump and awaited by its waiter.
#[derive(Debug, Default)]
pub(super) struct DrainSignal {
    progress: Mutex<Progress>,
    changed: Condvar,
}

impl DrainSignal {
    /// Records one completed read.
    pub(super) fn advance(&self) {
        {
            let mut progress = self.progress.lock().unwrap_or_else(PoisonError::into_inner);
            progress.reads = progress.reads.wrapping_add(1);
        }
        self.changed.notify_all();
    }

    /// Records that the pump will read no more.
    pub(super) fn finish(&self) {
        self.progress
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .finished = true;
        self.changed.notify_all();
    }

    /// Keeps the child unreaped until the pump has published its EOF evidence.
    pub(super) fn wait_until_finished(&self) {
        let progress = self.progress.lock().unwrap_or_else(PoisonError::into_inner);
        drop(
            self.changed
                .wait_while(progress, |now| !now.finished)
                .unwrap_or_else(PoisonError::into_inner),
        );
    }

    /// Blocks until `probe` has no unread output or the pump has finished.
    pub(super) fn wait_until_drained(&self, probe: &OwnedFd) -> io::Result<()> {
        let mut progress = self.progress.lock().unwrap_or_else(PoisonError::into_inner);
        while !progress.finished && pending_bytes(probe)? > 0 {
            let seen = progress.reads;
            progress = self
                .changed
                .wait_while(progress, |now| now.reads == seen && !now.finished)
                .unwrap_or_else(PoisonError::into_inner);
        }
        drop(progress);
        Ok(())
    }
}

fn pending_bytes(probe: &OwnedFd) -> io::Result<libc::c_int> {
    let mut pending: libc::c_int = 0;
    // SAFETY: `probe` is an open descriptor for the PTY master and FIONREAD writes
    // one c_int through the pointer to the live local `pending`.
    let status = unsafe { libc::ioctl(probe.as_raw_fd(), libc::FIONREAD, &raw mut pending) };
    if status == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(pending)
}
