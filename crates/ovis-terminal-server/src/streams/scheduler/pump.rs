//! Owned pump threads: one dedicated thread per blocking program-output reader.
//!
//! A pump reads until end of file and hands each read to its actor's bounded feed; a full
//! feed blocks the pump, which stops reading and so pushes back on the program. The
//! thread is owned and joined, never parked on the shared blocking pool.

use std::io::{ErrorKind, Read};
use std::thread::{self, JoinHandle};

use super::error::SchedulerError;
use super::handle::OutputFeed;

const READ_BYTES: usize = 16 * 1024;

/// Why a pump stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PumpEnd {
    /// The reader reported end of file, or the pseudo-terminal's EIO after hangup.
    Eof,
    /// The actor stopped taking output.
    ActorStopped,
    /// The reader failed with this error kind.
    Failed(ErrorKind),
}

/// What a pump carried before it stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PumpReport {
    /// Bytes handed to the actor.
    pub bytes: u64,
    /// Reads handed to the actor.
    pub reads: u64,
    /// Why the pump stopped.
    pub end: PumpEnd,
}

/// A running pump thread; join it once its reader ends.
#[derive(Debug)]
pub struct PumpThread(JoinHandle<PumpReport>);

impl PumpThread {
    /// Starts a thread named `name` that pumps `reader` into `feed`.
    ///
    /// # Errors
    ///
    /// Returns `internal` when the thread cannot start.
    pub fn spawn<R>(name: &str, reader: R, feed: OutputFeed) -> Result<Self, SchedulerError>
    where
        R: Read + Send + 'static,
    {
        thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || pump(reader, &feed))
            .map(Self)
            .map_err(SchedulerError::Spawn)
    }

    /// Waits for the pump to stop.
    ///
    /// Blocks the calling thread; call it from a blocking context.
    ///
    /// # Errors
    ///
    /// Returns `internal` when the pump thread panicked.
    pub fn join(self) -> Result<PumpReport, SchedulerError> {
        self.0.join().map_err(|_| SchedulerError::Panicked)
    }
}

/// EIO is how a pseudo-terminal master reports that every slave closed.
fn hung_up(error: &std::io::Error) -> bool {
    cfg!(unix) && error.raw_os_error() == Some(5)
}

fn pump<R: Read>(mut reader: R, feed: &OutputFeed) -> PumpReport {
    let mut buffer = vec![0_u8; READ_BYTES];
    let mut report = PumpReport {
        bytes: 0,
        reads: 0,
        end: PumpEnd::Eof,
    };
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) => return report,
            Ok(read) => read,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) if hung_up(&error) => return report,
            Err(error) => {
                report.end = PumpEnd::Failed(error.kind());
                return report;
            }
        };
        let bytes = buffer.get(..read).unwrap_or_default();
        if feed.blocking_send(bytes).is_err() {
            report.end = PumpEnd::ActorStopped;
            return report;
        }
        report.reads = report.reads.saturating_add(1);
        let read = u64::try_from(read).unwrap_or(u64::MAX);
        report.bytes = report.bytes.saturating_add(read);
    }
}
