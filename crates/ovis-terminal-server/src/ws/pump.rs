//! Moves frames between a WebSocket and the connection's byte stream.
//!
//! Reading and writing run concurrently, as the two directions of a socket do, so a
//! connection waiting to queue a reply never stops the peer's next request from being
//! read. Pings are answered on the transport and never reach the connection. The pump
//! ends when the connection stops writing or the close frame is sent; a peer violation
//! is answered with a close frame naming its kind.

use tokio::io::{
    AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, ReadHalf, WriteHalf,
};
use tokio::sync::mpsc;

use super::WsError;
use super::frame::{self, FrameDecoder, Inbound, Opcode};
use super::transport::{MessageStream, QUEUED_MESSAGES};
use crate::peer_gone;

const READ_CHUNK: usize = 64 * 1024;
const NORMAL_CLOSURE: u16 = 1000;
/// Pongs and the close reply waiting for the writer.
const QUEUED_CONTROLS: usize = 4;

/// A transport-level frame the reader asks the writer to send.
#[derive(Debug)]
enum Control {
    Pong(Vec<u8>),
    Close { code: u16, reason: &'static str },
}

/// The WebSocket side of one connection.
#[derive(Debug)]
pub(super) struct Pump<S> {
    socket: S,
    decoder: FrameDecoder,
    inbound: mpsc::Sender<Vec<u8>>,
    outbound: mpsc::Receiver<Vec<u8>>,
}

/// Splits an upgraded socket into the connection's byte stream and the pump feeding it;
/// `decoder` already holds the bytes read past the upgrade request.
pub(super) fn channel<S>(socket: S, decoder: FrameDecoder) -> (MessageStream, Pump<S>) {
    let (inbound, read) = mpsc::channel(QUEUED_MESSAGES);
    let (write, outbound) = mpsc::channel(QUEUED_MESSAGES);
    let pump = Pump {
        socket,
        decoder,
        inbound,
        outbound,
    };
    (MessageStream::new(read, write), pump)
}

impl<S: AsyncRead + AsyncWrite> Pump<S> {
    /// Runs until the connection stops writing or a close frame is sent.
    ///
    /// # Errors
    ///
    /// The peer's protocol violation, or a transport failure other than a vanished peer.
    pub(super) async fn run(self) -> Result<(), WsError> {
        let (reader, writer) = tokio::io::split(self.socket);
        let (control, controls) = mpsc::channel(QUEUED_CONTROLS);
        let reading = read_frames((reader, self.decoder), self.inbound, control);
        let writing = write_frames(writer, self.outbound, controls);
        tokio::pin!(reading, writing);
        let mut read = None;
        let written = loop {
            tokio::select! {
                ended = &mut reading, if read.is_none() => read = Some(ended),
                written = &mut writing => break written,
            }
        };
        // A violation the reader found outranks how the close frame went out.
        match read {
            Some(Err(violation)) => Err(violation),
            Some(Ok(())) | None => written,
        }
    }
}

async fn read_frames<S: AsyncRead>(
    (mut reader, mut decoder): (ReadHalf<S>, FrameDecoder),
    inbound: mpsc::Sender<Vec<u8>>,
    control: mpsc::Sender<Control>,
) -> Result<(), WsError> {
    let mut chunk = vec![0_u8; READ_CHUNK];
    let failure = loop {
        let frame = match decoder.next() {
            Ok(Some(frame)) => frame,
            Ok(None) => match reader.read(&mut chunk).await {
                Ok(0) => return Ok(()),
                Ok(count) => {
                    decoder.feed(chunk.get(..count).unwrap_or_default());
                    continue;
                }
                Err(error) if peer_gone(&error) => return Ok(()),
                Err(error) => break WsError::Transport(error),
            },
            Err(violation) => break violation,
        };
        let delivered = match frame {
            Inbound::Data(payload) => inbound.send(payload).await.is_ok(),
            Inbound::Ping(payload) => control.send(Control::Pong(payload)).await.is_ok(),
            Inbound::Pong => true,
            Inbound::Close(code) => {
                let code = code.unwrap_or(NORMAL_CLOSURE);
                let reply = Control::Close { code, reason: "" };
                // The writer may already have closed; the peer's close ends reading either way.
                return match control.send(reply).await {
                    Ok(()) | Err(_) => Ok(()),
                };
            }
        };
        if !delivered {
            return Ok(());
        }
    };
    let reason = failure.kind();
    let code = failure.close_code();
    match control.send(Control::Close { code, reason }).await {
        Ok(()) | Err(_) => Err(failure),
    }
}

async fn write_frames<S: AsyncWrite>(
    mut writer: WriteHalf<S>,
    mut outbound: mpsc::Receiver<Vec<u8>>,
    mut controls: mpsc::Receiver<Control>,
) -> Result<(), WsError> {
    let written = loop {
        let (bytes, last) = tokio::select! {
            biased;
            Some(control) = controls.recv() => match control {
                Control::Pong(payload) => (frame::encode(Opcode::Pong, &payload), false),
                Control::Close { code, reason } => (frame::close(code, reason), true),
            },
            next = outbound.recv() => next.map_or_else(
                || (frame::close(NORMAL_CLOSURE, ""), true),
                |payload| (frame::encode(Opcode::Binary, &payload), false),
            ),
        };
        if let Err(error) = writer.write_all(&bytes).await {
            break Err(error);
        }
        if last {
            break writer.shutdown().await;
        }
    };
    match written {
        Ok(()) => Ok(()),
        Err(error) if peer_gone(&error) => {
            // As on a socket whose peer left, the connection learns it from the end of its
            // input, not from a failed write: what it still writes is discarded.
            while outbound.recv().await.is_some() {}
            Ok(())
        }
        Err(error) => Err(WsError::Transport(error)),
    }
}
