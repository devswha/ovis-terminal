//! One core connection: decode requests, apply them, stream the core link back.
//!
//! The v1 holder wire has no error reply, so a rejected request ends the
//! connection with a typed, queryable reason; the core re-adopts to resynchronize.
//! Spawn failures and input overflow are not rejections: the holder already
//! answered them with `SpawnFailed` and a zero-byte `InputWritten`.

use std::io::{ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::PoisonError;
use std::sync::atomic::Ordering;
use std::thread::{Builder, JoinHandle};

use super::ServiceContext;
use crate::protocol::{Drain, FrameDecoder, GoodbyeReason, Message, encode_message};
use crate::unix::error::HolderError;
use crate::unix::link::{CoreLink, LinkClosed, RecvError};
use crate::unix::service_error::ConnectionError;

const READ_CHUNK: usize = 64 * 1024;

/// How a connection ended without a failure.
enum Flow {
    Continue,
    Ended,
    EndService,
}

struct Session<'context> {
    context: &'context ServiceContext,
    stream: UnixStream,
    link: Option<(CoreLink, JoinHandle<Result<(), ConnectionError>>)>,
}

pub(super) fn serve(context: &ServiceContext, stream: UnixStream) -> Result<(), ConnectionError> {
    let tracked = stream
        .try_clone()
        .map_err(|source| ConnectionError::Io { source })?;
    *context
        .current
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(tracked);
    let mut session = Session {
        context,
        stream,
        link: None,
    };
    let outcome = session.read_requests();
    *context
        .current
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = None;
    let writer = session.finish();
    outcome.and(writer)
}

impl Session<'_> {
    fn read_requests(&mut self) -> Result<(), ConnectionError> {
        let mut decoder = FrameDecoder::default();
        let mut buffer = vec![0_u8; READ_CHUNK];
        loop {
            let read = match self.stream.read(&mut buffer) {
                Ok(0) => return decoder.finish().map_err(ConnectionError::from),
                Ok(read) => read,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(source) => return Err(ConnectionError::Io { source }),
            };
            let chunk = buffer.get(..read).unwrap_or_default();
            for message in decoder.push(chunk)? {
                match self.dispatch(message)? {
                    Flow::Continue => {}
                    Flow::Ended => return Ok(()),
                    Flow::EndService => {
                        self.context.stopping.store(true, Ordering::SeqCst);
                        return Ok(());
                    }
                }
            }
        }
    }

    fn adopt(&mut self, message: Message) -> Result<Flow, ConnectionError> {
        let Message::Hello(hello) = message else {
            return Err(ConnectionError::HelloRequired {
                kind: message.kind(),
            });
        };
        let link = self.context.holder.adopt(&hello)?;
        self.start_writer(link)?;
        Ok(Flow::Continue)
    }

    fn dispatch(&mut self, message: Message) -> Result<Flow, ConnectionError> {
        let Some(link) = self.link.as_ref().map(|(link, _)| link.clone()) else {
            return self.adopt(message);
        };
        let holder = &self.context.holder;
        match message {
            Message::Spawn(spawn) => match holder.spawn_wire(&spawn) {
                Ok(_)
                | Err(
                    HolderError::Launch { .. }
                    | HolderError::Draining
                    | HolderError::WorkerUnavailable { .. },
                ) => {}
                Err(error) => return Err(error.into()),
            },
            Message::Input(input) => match holder.input(input) {
                Ok(()) | Err(HolderError::InputOverflow { .. }) => {}
                Err(error) => return Err(error.into()),
            },
            Message::Resize(resize) => holder.resize(&resize)?,
            Message::Signal(signal) => holder.signal(&signal)?,
            Message::TailRequest(request) => {
                let tail = holder.tail(&request)?;
                link.outbox()
                    .push(Message::Tail(tail))
                    .map_err(|reason| HolderError::LinkClosed { reason })?;
            }
            Message::Forget(forget) => holder.forget(&forget)?,
            Message::Drain(Drain::Begin) => holder.begin_drain(),
            Message::Drain(Drain::Complete) if holder.is_empty() => return Ok(Flow::EndService),
            Message::Drain(Drain::Complete) => return Err(ConnectionError::DrainIncomplete),
            Message::Goodbye(goodbye) => {
                return Ok(match goodbye.reason {
                    GoodbyeReason::Handover => Flow::Ended,
                    GoodbyeReason::Shutdown => Flow::EndService,
                });
            }
            other @ (Message::Hello(_)
            | Message::Welcome(_)
            | Message::Spawned(_)
            | Message::SpawnFailed(_)
            | Message::Output(_)
            | Message::InputWritten(_)
            | Message::Exit(_)
            | Message::Tail(_)) => {
                return Err(ConnectionError::UnexpectedMessage { kind: other.kind() });
            }
        }
        Ok(Flow::Continue)
    }

    fn start_writer(&mut self, link: CoreLink) -> Result<(), ConnectionError> {
        let stream = self
            .stream
            .try_clone()
            .map_err(|source| ConnectionError::Io { source })?;
        let writer_link = link.clone();
        let writer = Builder::new()
            .name("ovis-holder-link".to_owned())
            .spawn(move || write_link(&writer_link, stream))
            .map_err(|source| ConnectionError::Io { source })?;
        self.link = Some((link, writer));
        Ok(())
    }

    /// Releases the link and joins its writer; the holder keeps every terminal.
    fn finish(self) -> Result<(), ConnectionError> {
        let Some((link, writer)) = self.link else {
            return Ok(());
        };
        link.release();
        // The peer may have closed already; shutting down an ended socket is moot.
        self.stream.shutdown(Shutdown::Both).unwrap_or_default();
        writer.join().map_err(|_| ConnectionError::WriterPanicked)?
    }
}

/// Streams link messages to the core until the link closes or the socket fails.
fn write_link(link: &CoreLink, mut stream: UnixStream) -> Result<(), ConnectionError> {
    let result = loop {
        let message = match link.recv() {
            Ok(message) => message,
            Err(RecvError::Closed {
                reason: LinkClosed::Overflow,
            }) => break Err(ConnectionError::LinkOverflow),
            Err(RecvError::Closed { .. } | RecvError::Timeout) => break Ok(()),
        };
        let written = encode_message(&message)
            .map_err(ConnectionError::from)
            .and_then(|frame| {
                stream
                    .write_all(&frame)
                    .map_err(|source| ConnectionError::Io { source })
            });
        if let Err(error) = written {
            link.release();
            break Err(error);
        }
    };
    // Unblock the reader when the holder closed the link (replaced, overflow, stop).
    stream.shutdown(Shutdown::Both).unwrap_or_default();
    result
}
