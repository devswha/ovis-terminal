//! Terminal output as the probe consumes it: bounded pending bytes, marker
//! search and end-of-stream detection.

use std::mem;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::error::ProbeError;
use super::measure::ByteCount;
use super::pump::PumpEvent;

/// Output held while searching for an expected marker.
const PENDING_LIMIT_BYTES: u64 = 64 * 1024 * 1024;

/// How the output stream ended.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StreamEnd {
    /// The backend reported end of file.
    Eof,
    /// The backend reported a read error, named by its OS error kind.
    ReadError(String),
}

#[derive(Debug)]
pub(super) struct OutputStream {
    events: Receiver<PumpEvent>,
    pending: Vec<u8>,
    received: ByteCount,
    end: Option<StreamEnd>,
}

impl OutputStream {
    pub(super) const fn new(events: Receiver<PumpEvent>) -> Self {
        Self {
            events,
            pending: Vec::new(),
            received: ByteCount::new(0),
            end: None,
        }
    }

    pub(super) const fn received(&self) -> ByteCount {
        self.received
    }

    pub(super) fn pending(&self) -> &[u8] {
        &self.pending
    }

    pub(super) fn read_until(
        &mut self,
        needle: &[u8],
        bound: Duration,
    ) -> Result<Vec<u8>, ProbeError> {
        let deadline = Instant::now().checked_add(bound);
        let mut searched = 0;
        loop {
            let window = self.pending.get(searched..).unwrap_or_default();
            if let Some(found) = window
                .windows(needle.len())
                .position(|candidate| candidate == needle)
            {
                let end = searched.saturating_add(found).saturating_add(needle.len());
                return Ok(self.pending.drain(..end).collect());
            }
            searched = self
                .pending
                .len()
                .saturating_sub(needle.len().saturating_sub(1));
            if self.end.is_some() {
                return Err(ProbeError::OutputEnded);
            }
            if u64::try_from(self.pending.len()).unwrap_or(u64::MAX) > PENDING_LIMIT_BYTES {
                return Err(ProbeError::OutputLimitExceeded {
                    limit: ByteCount::new(PENDING_LIMIT_BYTES),
                });
            }
            self.absorb_next(deadline, bound)?;
        }
    }

    pub(super) fn drain_to_end(
        &mut self,
        bound: Duration,
    ) -> Result<(Vec<u8>, StreamEnd), ProbeError> {
        let deadline = Instant::now().checked_add(bound);
        loop {
            if let Some(end) = self.end.clone() {
                return Ok((mem::take(&mut self.pending), end));
            }
            self.absorb_next(deadline, bound)?;
        }
    }

    fn absorb_next(
        &mut self,
        deadline: Option<Instant>,
        bound: Duration,
    ) -> Result<(), ProbeError> {
        let left = deadline.map_or(bound, |at| at.saturating_duration_since(Instant::now()));
        match self.events.recv_timeout(left) {
            Ok(PumpEvent::Bytes(bytes)) => {
                self.received = self
                    .received
                    .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
                self.pending.extend_from_slice(&bytes);
            }
            Ok(PumpEvent::Eof) | Err(RecvTimeoutError::Disconnected) => {
                self.end = Some(StreamEnd::Eof);
            }
            Ok(PumpEvent::Failed(error)) => {
                self.end = Some(StreamEnd::ReadError(error.kind().to_string()));
            }
            Err(RecvTimeoutError::Timeout) => return Err(ProbeError::Timeout { waited: bound }),
        }
        Ok(())
    }
}
