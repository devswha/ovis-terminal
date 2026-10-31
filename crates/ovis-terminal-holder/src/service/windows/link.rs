//! One core connection to a Windows holder: admit the peer, adopt, serve requests.
//!
//! The peer must send within a bound; its account, read by impersonation once its first
//! bytes arrived, must be the holder's owner even though the pipe DACL already refused
//! everyone else. `Welcome` is queued before the link is attached, so it precedes every
//! output record. Ending a connection never ends a terminal.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::windows::named_pipe::NamedPipeServer;
use tokio::sync::mpsc;

use super::pipe;
use super::terminals::{Outbox, Table};
use crate::protocol::{
    AdoptionTable, Drain, FrameDecoder, GoodbyeReason, HolderDescriptor, Message, encode_message,
    negotiate,
};

mod error;

pub use error::LinkError;

const ADMISSION_BOUND: Duration = Duration::from_secs(10);
const READ_CHUNK: usize = 64 * 1024;

/// What the holder serves every connection with.
#[derive(Debug)]
pub(super) struct LinkContext {
    pub(super) owner: String,
    pub(super) descriptor: HolderDescriptor,
    pub(super) table: Arc<Table>,
}

/// How a connection ended without a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Flow {
    /// The core left; the holder keeps serving.
    Ended,
    /// The core asked the holder to stop.
    EndService,
}

async fn admit(pipe: &NamedPipeServer, owner: &str) -> Result<(), LinkError> {
    let bound_ms = u64::try_from(ADMISSION_BOUND.as_millis()).unwrap_or(u64::MAX);
    tokio::time::timeout(ADMISSION_BOUND, pipe.readable())
        .await
        .map_err(|_| LinkError::PeerSilent { bound_ms })?
        .map_err(LinkError::Io)?;
    if pipe::client_sid(pipe).map_err(LinkError::Io)? != owner {
        return Err(LinkError::PeerRejected);
    }
    Ok(())
}

fn adopt(message: Message, context: &LinkContext) -> Result<Message, LinkError> {
    let Message::Hello(hello) = message else {
        return Err(LinkError::HelloRequired {
            kind: message.kind(),
        });
    };
    let mut records = AdoptionTable::default();
    for held in context.table.held() {
        records.retain(held)?;
    }
    Ok(Message::Welcome(negotiate(
        &hello,
        &context.descriptor,
        &records,
    )?))
}

fn dispatch(
    message: Message,
    table: &Arc<Table>,
    outbox: &Outbox,
) -> Result<Option<Flow>, LinkError> {
    match message {
        Message::Spawn(spawn) => table.spawn(&spawn)?,
        Message::Input(input) => table.input(input)?,
        Message::Resize(resize) => table.resize(&resize)?,
        Message::Signal(signal) => table.signal(&signal)?,
        Message::TailRequest(request) => {
            outbox
                .send(Message::Tail(table.tail(&request)?))
                .map_err(|_| writer_closed())?;
        }
        Message::Forget(forget) => table.forget(&forget)?,
        Message::Drain(Drain::Begin) => table.begin_drain(),
        Message::Drain(Drain::Complete) if table.is_empty() => return Ok(Some(Flow::EndService)),
        Message::Drain(Drain::Complete) => return Err(LinkError::DrainIncomplete),
        Message::Goodbye(goodbye) => {
            return Ok(Some(match goodbye.reason {
                GoodbyeReason::Handover => Flow::Ended,
                GoodbyeReason::Shutdown => Flow::EndService,
            }));
        }
        other @ (Message::Hello(_)
        | Message::Welcome(_)
        | Message::Spawned(_)
        | Message::SpawnFailed(_)
        | Message::Output(_)
        | Message::InputWritten(_)
        | Message::Exit(_)
        | Message::Tail(_)
        | Message::InputRejected(_)) => {
            return Err(LinkError::UnexpectedMessage { kind: other.kind() });
        }
    }
    Ok(None)
}

fn writer_closed() -> LinkError {
    LinkError::Io(std::io::Error::new(
        std::io::ErrorKind::BrokenPipe,
        "the link writer ended",
    ))
}

/// A connection before and after adoption.
enum State {
    Adopting(tokio::io::WriteHalf<NamedPipeServer>),
    Serving(Outbox, tokio::task::JoinHandle<Result<(), LinkError>>),
}

impl State {
    fn step(
        self,
        message: Message,
        context: &LinkContext,
    ) -> (Self, Result<Option<Flow>, LinkError>) {
        let table = &context.table;
        match self {
            Self::Serving(outbox, writing) => {
                let step = dispatch(message, table, &outbox);
                (Self::Serving(outbox, writing), step)
            }
            Self::Adopting(half) => match adopt(message, context) {
                Err(error) => (Self::Adopting(half), Err(error)),
                Ok(welcome) => {
                    let (outbox, inbox) = mpsc::unbounded_channel();
                    let queued = outbox.send(welcome).map_err(|_| writer_closed());
                    table.attach(outbox.clone());
                    let writing = tokio::spawn(write_link(half, inbox));
                    (Self::Serving(outbox, writing), queued.map(|()| None))
                }
            },
        }
    }
}

async fn write_link(
    mut writer: tokio::io::WriteHalf<NamedPipeServer>,
    mut inbox: mpsc::UnboundedReceiver<Message>,
) -> Result<(), LinkError> {
    while let Some(message) = inbox.recv().await {
        let frame = encode_message(&message)?;
        writer.write_all(&frame).await.map_err(LinkError::Io)?;
    }
    writer.shutdown().await.map_err(LinkError::Io)
}

/// Serves one accepted connection until the core leaves or the connection fails.
pub(super) async fn serve(
    pipe: NamedPipeServer,
    context: Arc<LinkContext>,
) -> Result<Flow, LinkError> {
    admit(&pipe, &context.owner).await?;
    let (mut reader, writer) = tokio::io::split(pipe);
    let mut decoder = FrameDecoder::default();
    let mut buffer = vec![0_u8; READ_CHUNK];
    let mut state = State::Adopting(writer);
    let flow = loop {
        let read = match reader.read(&mut buffer).await {
            Ok(0) => {
                break decoder
                    .finish()
                    .map(|()| Flow::Ended)
                    .map_err(LinkError::from);
            }
            Ok(read) => read,
            Err(error) => break Err(LinkError::Io(error)),
        };
        let messages = match decoder.push(buffer.get(..read).unwrap_or_default()) {
            Ok(messages) => messages,
            Err(error) => break Err(error.into()),
        };
        let mut ended = None;
        for message in messages {
            let (next, step) = state.step(message, &context);
            state = next;
            match step {
                Ok(None) => {}
                other => {
                    ended = Some(other.map(|flow| flow.unwrap_or(Flow::Ended)));
                    break;
                }
            }
        }
        if let Some(ended) = ended {
            break ended;
        }
    };
    if let State::Serving(outbox, writing) = state {
        context.table.detach(&outbox);
        drop(outbox);
        let written = writing
            .await
            .map_err(|error| LinkError::Io(std::io::Error::other(error)))?;
        written?;
    }
    flow
}
