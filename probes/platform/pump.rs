//! Threads that move blocking pseudo-terminal reads and waits off the caller.

use std::io::{self, Read};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::{self, JoinHandle};

use portable_pty::{Child, ExitStatus};

use super::error::{ProbeError, PtyOperation};

/// Chunks queued between the reader thread and the session; a full queue
/// stops the reader, which leaves the bytes in the kernel buffer.
const OUTPUT_QUEUE_CHUNKS: usize = 64;
const READ_CHUNK_BYTES: usize = 16 * 1024;

#[derive(Debug)]
pub(super) enum PumpEvent {
    Bytes(Vec<u8>),
    Eof,
    Failed(io::Error),
}

pub(super) fn spawn_reader(
    reader: Box<dyn Read + Send>,
) -> Result<(Receiver<PumpEvent>, JoinHandle<()>), ProbeError> {
    let (sender, receiver) = sync_channel(OUTPUT_QUEUE_CHUNKS);
    let handle = thread::Builder::new()
        .name("probe-pty-reader".into())
        .spawn(move || pump(reader, &sender))
        .map_err(|source| ProbeError::Io {
            operation: PtyOperation::StartThread,
            source,
        })?;
    Ok((receiver, handle))
}

fn pump(mut reader: Box<dyn Read + Send>, sender: &SyncSender<PumpEvent>) {
    let mut buffer = vec![0_u8; READ_CHUNK_BYTES];
    loop {
        let event = match reader.read(&mut buffer) {
            Ok(0) => PumpEvent::Eof,
            Ok(read) => PumpEvent::Bytes(buffer.get(..read).unwrap_or_default().to_vec()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => PumpEvent::Failed(error),
        };
        let last = !matches!(event, PumpEvent::Bytes(_));
        if sender.send(event).is_err() || last {
            return;
        }
    }
}

pub(super) fn spawn_waiter(
    mut child: Box<dyn Child + Send + Sync>,
) -> Result<(Receiver<io::Result<ExitStatus>>, JoinHandle<()>), ProbeError> {
    let (sender, receiver) = sync_channel(1);
    let handle = thread::Builder::new()
        .name("probe-pty-waiter".into())
        // The session may already be gone; the status then has no reader.
        .spawn(move || drop(sender.send(child.wait())))
        .map_err(|source| ProbeError::Io {
            operation: PtyOperation::StartThread,
            source,
        })?;
    Ok((receiver, handle))
}
