//! The harness's own viewer of a control boot. An interruption cuts its connection and
//! resumes from the next expected sequence; every sequence is accounted for as received,
//! covered by an explicit gap record, or a hole.

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::Instant;

use ovis_terminal_protocol::schema::scalar::EventSeq;
use serde::Serialize;

use super::error::SoakError;
use super::wire::{Hello, Line, Request};

/// Sequence accounting across every connection to one boot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(super) struct Continuity {
    pub(super) received: u64,
    /// Sequences an explicit gap record covered.
    pub(super) gapped: u64,
    /// Sequences that were skipped without any record: never acceptable.
    pub(super) holes: u64,
    pub(super) resumes: u64,
}

#[derive(Debug)]
pub(super) struct Observer {
    socket: PathBuf,
    name: String,
    writer: UnixStream,
    lines: Receiver<Arrival>,
    reader: JoinHandle<bool>,
    /// The next sequence this observer expects.
    next: u64,
    pub(super) continuity: Continuity,
    /// When the line most recently returned by [`Observer::next`] arrived.
    pub(super) arrived: Instant,
}

/// One received line and when it arrived, stamped by the reader thread so a busy harness
/// never inflates a measured latency.
type Arrival = (Instant, Option<Line>);

/// Forwards parsed lines; `None` marks the end. `false` once nobody listens.
fn read(stream: UnixStream, sender: &std::sync::mpsc::Sender<Arrival>) -> bool {
    for line in BufReader::new(stream).lines() {
        let parsed = line
            .ok()
            .and_then(|text| serde_json::from_str::<Line>(&text).ok());
        let stop = parsed.is_none();
        if sender.send((Instant::now(), parsed)).is_err() {
            return false;
        }
        if stop {
            return true;
        }
    }
    sender.send((Instant::now(), None)).is_ok()
}

type Opened = (UnixStream, Receiver<Arrival>, JoinHandle<bool>);

impl Observer {
    fn open(socket: &Path, hello: &Hello) -> Result<Opened, SoakError> {
        let mut writer =
            UnixStream::connect(socket).map_err(SoakError::io("connect to the control socket"))?;
        let mut line = serde_json::to_string(hello)?;
        line.push('\n');
        writer
            .write_all(line.as_bytes())
            .map_err(SoakError::io("send the viewer hello"))?;
        let reading = writer
            .try_clone()
            .map_err(SoakError::io("clone the viewer stream"))?;
        let (sender, lines) = std::sync::mpsc::channel();
        let reader = std::thread::Builder::new()
            .name("soak-observer".to_owned())
            .spawn(move || read(reading, &sender))
            .map_err(SoakError::io("start the observer reader"))?;
        Ok((writer, lines, reader))
    }

    /// A new observer of the boot serving `socket`, replaying what it retains.
    pub(super) fn connect(socket: &Path, name: &str) -> Result<Self, SoakError> {
        let hello = Hello {
            viewer: name.to_owned(),
            reads: true,
            from_seq: None,
        };
        let (writer, lines, reader) = Self::open(socket, &hello)?;
        Ok(Self {
            socket: socket.to_path_buf(),
            name: name.to_owned(),
            writer,
            lines,
            reader,
            next: 0,
            continuity: Continuity::default(),
            arrived: Instant::now(),
        })
    }

    /// The scheduled interruption: the connection is cut, then resumed at the next
    /// expected sequence of the same boot.
    pub(super) fn interrupt(self) -> Result<Self, SoakError> {
        let Self {
            socket,
            name,
            writer,
            lines,
            reader,
            next,
            mut continuity,
            ..
        } = self;
        writer
            .shutdown(Shutdown::Both)
            .map_err(SoakError::io("cut the observer connection"))?;
        reader.join().map_err(|_| SoakError::Protocol {
            expected: "an observer reader that ends",
        })?;
        // Lines still queued from the cut connection are replayed from `next` below.
        drop(lines);
        let hello = Hello {
            viewer: name.clone(),
            reads: true,
            from_seq: Some(EventSeq::new(next)),
        };
        let (writer, lines, reader) = Self::open(&socket, &hello)?;
        continuity.resumes = continuity.resumes.saturating_add(1);
        Ok(Self {
            socket,
            name,
            writer,
            lines,
            reader,
            next,
            continuity,
            arrived: Instant::now(),
        })
    }

    pub(super) fn send(&mut self, request: Request) -> Result<(), SoakError> {
        let mut line = serde_json::to_string(&request)?;
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .map_err(SoakError::io("send a viewer request"))
    }

    /// Counts `line` against the expected sequence; `false` for a duplicate event.
    const fn account(&mut self, line: &Line) -> bool {
        match line {
            Line::Event { seq, .. } => {
                let seq = seq.get();
                if seq < self.next {
                    return false;
                }
                let skipped = seq.saturating_sub(self.next);
                self.continuity.holes = self.continuity.holes.saturating_add(skipped);
                self.continuity.received = self.continuity.received.saturating_add(1);
                self.next = seq.saturating_add(1);
                true
            }
            Line::Gap { from, to } => {
                if from.get() <= self.next && to.get() > self.next {
                    let covered = to.get().saturating_sub(self.next);
                    self.continuity.gapped = self.continuity.gapped.saturating_add(covered);
                    self.next = to.get();
                }
                true
            }
            Line::Pong { .. } | Line::Status { .. } => true,
        }
    }

    /// The next line before `deadline`; `None` at the deadline. A closed connection is
    /// [`SoakError::ControlGone`].
    pub(super) fn next(&mut self, deadline: Instant) -> Result<Option<Line>, SoakError> {
        loop {
            let wait = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(wait) {
                Ok((at, Some(line))) => {
                    if self.account(&line) {
                        self.arrived = at;
                        return Ok(Some(line));
                    }
                }
                Ok((_, None)) | Err(RecvTimeoutError::Disconnected) => {
                    return Err(SoakError::ControlGone {
                        waiting_for: "observer lines",
                    });
                }
                Err(RecvTimeoutError::Timeout) => return Ok(None),
            }
        }
    }

    /// Ends the connection and joins its reader.
    pub(super) fn close(self) -> Result<Continuity, SoakError> {
        match self.writer.shutdown(Shutdown::Both) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotConnected => {}
            Err(source) => return Err(SoakError::io("close the observer")(source)),
        }
        self.reader.join().map_err(|_| SoakError::Protocol {
            expected: "an observer reader that ends",
        })?;
        Ok(self.continuity)
    }
}
