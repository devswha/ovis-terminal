//! One protocol connection with a byte-exact transcript and decoded-byte accounting.

use std::collections::VecDeque;

use ovis_terminal_protocol::wire::{
    Channel, ConnectionDecoder, FRAME_HEADER_LEN, FeedOutcome, Frame, FrameKind, WireMessage,
};
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::time::Instant;

use super::redact::scrub;
use super::transport::{Bridge, Opened, Reader, Writer};
use crate::error::RunnerError;
use crate::evidence::{ConnectionReport, Direction, ProcessReport, Record, TEXT_LIMIT, sha256_hex};
use crate::ids::Decimal;
use crate::scenario::model::Split;

const READ_BUFFER: usize = 64 * 1024;
pub(crate) const REDACTED: &str = "<redacted-credential>";

/// Whether records are NDJSON lines or channel-0 frames after a framed hello.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Lines,
    Frames,
}

/// A bounded wait for the next record.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Wait {
    pub(crate) until: Instant,
    pub(crate) bound_ms: u64,
    pub(crate) waiting_for: &'static str,
}

impl Wait {
    pub(crate) fn within(bound_ms: u64, waiting_for: &'static str) -> Self {
        Self {
            until: Instant::now()
                .checked_add(std::time::Duration::from_millis(bound_ms))
                .unwrap_or_else(Instant::now),
            bound_ms,
            waiting_for,
        }
    }
}

/// A live connection; dropping it closes the transport and kills its bridge process.
pub(crate) struct Connection {
    reader: Reader,
    writer: Option<Writer>,
    bridge: Option<Bridge>,
    decoder: ConnectionDecoder,
    carry: Vec<u8>,
    mode: Mode,
    clock: Instant,
    secret: String,
    stashed: VecDeque<Value>,
    last_record_bytes: usize,
    report: ConnectionReport,
}

impl Connection {
    /// Wraps an opened transport; `secret` is scrubbed from every logged record.
    pub(crate) fn new(opened: Opened, report: ConnectionReport, secret: String) -> Self {
        Self {
            reader: opened.reader,
            writer: Some(opened.writer),
            bridge: opened.bridge,
            decoder: ConnectionDecoder::new(),
            carry: Vec::new(),
            mode: Mode::Lines,
            clock: Instant::now(),
            secret,
            stashed: VecDeque::new(),
            last_record_bytes: 0,
            report,
        }
    }

    /// The bearer this connection presents in its hello.
    pub(crate) fn secret(&self) -> &str {
        &self.secret
    }

    /// Encodes one JSON-RPC record for the current mode.
    pub(crate) fn encode(&self, record: &Value) -> Result<Vec<u8>, RunnerError> {
        let mut bytes = serde_json::to_vec(record)?;
        match self.mode {
            Mode::Lines => {
                bytes.push(b'\n');
                Ok(bytes)
            }
            Mode::Frames => Ok(Frame::new(Channel::CONTROL, FrameKind::Data, bytes)?.encode()?),
        }
    }

    /// Writes `bytes` cut exactly as `split` says, flushing after every write.
    pub(crate) async fn write(&mut self, bytes: &[u8], split: &Split) -> Result<(), RunnerError> {
        let sizes: Vec<usize> = match split {
            Split::Whole => vec![bytes.len().max(1)],
            Split::Bytewise => vec![1],
            Split::Chunks(sizes) => sizes.iter().copied().filter(|size| *size > 0).collect(),
        };
        let mut rest = bytes;
        let mut cycle = sizes.iter().cycle();
        while !rest.is_empty() {
            let size = cycle.next().copied().unwrap_or(rest.len()).min(rest.len());
            let (chunk, tail) = rest.split_at(size);
            self.write_once(chunk).await?;
            rest = tail;
        }
        Ok(())
    }

    async fn write_once(&mut self, chunk: &[u8]) -> Result<(), RunnerError> {
        let io = |source| RunnerError::Io {
            operation: "write to the connection",
            source,
        };
        let writer = self.writer.as_mut().ok_or(RunnerError::Closed {
            waiting_for: "a writable connection",
        })?;
        writer.write_all(chunk).await.map_err(io)?;
        writer.flush().await.map_err(io)?;
        let offset = self.report.bytes_sent;
        self.report.bytes_sent =
            Decimal(offset.0.saturating_add(Decimal::from_usize(chunk.len()).0));
        self.report.writes = Decimal(self.report.writes.0.saturating_add(1));
        let record = self.record(chunk, (Direction::Sent, "write".to_owned()));
        self.report.transcript.push(Record { offset, ..record });
        Ok(())
    }

    /// Switches to frames right after the framed hello reply, as the server does.
    pub(crate) fn upgrade(&mut self) -> Result<(), RunnerError> {
        self.decoder.upgrade_to_framed()?;
        self.mode = Mode::Frames;
        Ok(())
    }

    /// Puts a record back for a later step to consume.
    pub(crate) fn stash(&mut self, record: Value) {
        self.stashed.push_back(record);
    }

    /// Puts records a step read past back in front of everything still unread, in order.
    pub(crate) fn unread(&mut self, records: Vec<Value>) {
        for record in records.into_iter().rev() {
            self.stashed.push_front(record);
        }
    }

    /// Size on the wire of the most recently decoded record.
    pub(crate) const fn last_record_bytes(&self) -> usize {
        self.last_record_bytes
    }

    /// The next record, or `None` at end of stream; fails when the wait's bound passes first.
    pub(crate) async fn next(&mut self, wait: Wait) -> Result<Option<Value>, RunnerError> {
        if let Some(record) = self.stashed.pop_front() {
            return Ok(Some(record));
        }
        let mut buffer = vec![0; READ_BUFFER];
        loop {
            if let Some(message) = self.decoder.next_message()? {
                return self.decoded(message).map(Some);
            }
            if !self.carry.is_empty() {
                let pending = std::mem::take(&mut self.carry);
                self.feed(&pending)?;
                continue;
            }
            let read = tokio::time::timeout_at(wait.until, self.reader.read(&mut buffer)).await;
            let read = read
                .map_err(|_| RunnerError::Timeout {
                    waiting_for: wait.waiting_for,
                    bound_ms: wait.bound_ms,
                })?
                .map_err(|source| RunnerError::Io {
                    operation: "read from the connection",
                    source,
                })?;
            let Some(bytes) = buffer.get(..read).filter(|bytes| !bytes.is_empty()) else {
                return Ok(None);
            };
            let received = self.report.bytes_received.0;
            self.report.bytes_received =
                Decimal(received.saturating_add(Decimal::from_usize(read).0));
            let bytes = bytes.to_vec();
            self.feed(&bytes)?;
        }
    }

    fn feed(&mut self, bytes: &[u8]) -> Result<(), RunnerError> {
        if let FeedOutcome::NeedDrain { consumed } = self.decoder.feed(bytes)? {
            self.carry = bytes.get(consumed..).unwrap_or_default().to_vec();
        }
        Ok(())
    }

    fn decoded(&mut self, message: WireMessage) -> Result<Value, RunnerError> {
        let (payload, unit, framing) = match message {
            WireMessage::ControlLine(line) => (line, "line".to_owned(), 1),
            WireMessage::Frame(frame)
                if frame.channel == Channel::CONTROL && frame.kind == FrameKind::Data =>
            {
                (frame.payload, "frame:0:data".to_owned(), FRAME_HEADER_LEN)
            }
            _ => {
                return Err(RunnerError::Unexpected {
                    waiting_for: "a channel-0 control record",
                });
            }
        };
        let encoded = payload.len().saturating_add(framing);
        let offset = self.report.bytes_decoded;
        self.report.bytes_decoded =
            Decimal(offset.0.saturating_add(Decimal::from_usize(encoded).0));
        self.report.records_received = Decimal(self.report.records_received.0.saturating_add(1));
        self.last_record_bytes = encoded;
        let record = self.record(&payload, (Direction::Received, unit));
        self.report.transcript.push(Record {
            offset,
            bytes: Decimal::from_usize(encoded),
            ..record
        });
        Ok(serde_json::from_slice(&payload)?)
    }

    fn record(&self, bytes: &[u8], (direction, unit): (Direction, String)) -> Record {
        let at = u64::try_from(self.clock.elapsed().as_millis()).unwrap_or(u64::MAX);
        let text = (bytes.len() <= TEXT_LIMIT)
            .then(|| scrub(&String::from_utf8_lossy(bytes).replace(&self.secret, REDACTED)));
        Record {
            direction,
            offset: Decimal(0),
            bytes: Decimal::from_usize(bytes.len()),
            sha256: sha256_hex(bytes),
            at_ms: Decimal(at),
            unit,
            text,
        }
    }

    /// Every received byte has been decoded into a record: nothing is buffered or lost.
    pub(crate) const fn accounted(&self) -> (u64, u64) {
        (self.report.bytes_received.0, self.report.bytes_decoded.0)
    }

    /// Closes this side's write half; the peer sees end of stream.
    pub(crate) async fn hangup(&mut self) -> Result<(), RunnerError> {
        if let Some(mut writer) = self.writer.take() {
            writer.shutdown().await.map_err(|source| RunnerError::Io {
                operation: "close the write half",
                source,
            })?;
        }
        Ok(())
    }

    /// Closes the connection and reaps its bridge, returning the evidence of both.
    pub(crate) async fn finish(mut self) -> (ConnectionReport, Option<ProcessReport>) {
        drop(self.writer.take());
        let process = match self.bridge.take() {
            Some(bridge) => Some(bridge.reap().await),
            None => None,
        };
        (self.report, process)
    }
}

#[cfg(test)]
mod tests;
