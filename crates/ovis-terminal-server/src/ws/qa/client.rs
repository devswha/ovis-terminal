//! A raw RFC 6455 client written independently of the server's codec: it sends the
//! upgrade request and masked frames byte by byte and decodes unmasked server frames.

use std::net::SocketAddr;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sha1::{Digest as _, Sha1};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;

use super::TestResult;

const WAIT: Duration = Duration::from_secs(15);
const MASK: [u8; 4] = [0xA5, 0x3C, 0x0F, 0x96];

/// Opcodes the tests send and receive.
pub(crate) const BINARY: u8 = 0x2;
pub(crate) const TEXT: u8 = 0x1;
pub(crate) const PING: u8 = 0x9;

/// One server frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Received {
    Binary(Vec<u8>),
    Pong(Vec<u8>),
    Close(Option<u16>, String),
}

#[derive(Debug)]
pub(crate) struct WsClient {
    stream: TcpStream,
    pending: Vec<u8>,
}

impl WsClient {
    /// Upgrades a TCP connection; `Err` carries a refusal's whole HTTP response.
    pub(crate) async fn connect(
        address: SocketAddr,
        origin: Option<&str>,
    ) -> TestResult<Result<Self, String>> {
        let mut stream = TcpStream::connect(address).await?;
        let key = STANDARD.encode(uuid::Uuid::now_v7().as_bytes());
        let origin = origin.map(|origin| format!("Origin: {origin}\r\n"));
        let request = format!(
            "GET /ovis-terminal HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\n\
             Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\
             {}\r\n",
            origin.unwrap_or_default()
        );
        stream.write_all(request.as_bytes()).await?;
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let byte = tokio::time::timeout(WAIT, stream.read_u8()).await??;
            head.push(byte);
        }
        let head = String::from_utf8(head)?;
        if !head.starts_with("HTTP/1.1 101 ") {
            let mut body = String::new();
            tokio::time::timeout(WAIT, stream.read_to_string(&mut body)).await??;
            return Ok(Err(format!("{head}{body}")));
        }
        let mut hasher = Sha1::new();
        hasher.update(key.as_bytes());
        hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
        let accept = format!(
            "Sec-WebSocket-Accept: {}\r\n",
            STANDARD.encode(hasher.finalize())
        );
        if !head.contains(&accept) {
            return Err(format!("wrong accept key in {head}").into());
        }
        Ok(Ok(Self {
            stream,
            pending: Vec::new(),
        }))
    }

    /// Sends one masked, unfragmented frame.
    pub(crate) async fn send(&mut self, opcode: u8, payload: &[u8]) -> TestResult {
        let frame = masked(0x80 | opcode, payload)?;
        self.send_raw(&frame).await
    }

    /// Writes bytes verbatim.
    pub(crate) async fn send_raw(&mut self, bytes: &[u8]) -> TestResult {
        Ok(self.stream.write_all(bytes).await?)
    }

    /// The next server frame, or `None` once the server closed the TCP stream.
    pub(crate) async fn next(&mut self) -> TestResult<Option<Received>> {
        loop {
            if let Some((received, used)) = decode(&self.pending)? {
                self.pending.drain(..used);
                return Ok(Some(received));
            }
            let mut chunk = [0_u8; 4096];
            let read = tokio::time::timeout(WAIT, self.stream.read(&mut chunk)).await??;
            if read == 0 {
                return Ok(None);
            }
            self.pending
                .extend_from_slice(chunk.get(..read).ok_or("read past the buffer")?);
        }
    }
}

/// A client frame with `first` as its FIN/RSV/opcode byte.
pub(crate) fn masked(first: u8, payload: &[u8]) -> TestResult<Vec<u8>> {
    let mut frame = vec![first];
    if payload.len() < 126 {
        frame.push(0x80 | u8::try_from(payload.len())?);
    } else if let Ok(medium) = u16::try_from(payload.len()) {
        frame.push(0x80 | 0x7E);
        frame.extend_from_slice(&medium.to_be_bytes());
    } else {
        frame.push(0x80 | 0x7F);
        frame.extend_from_slice(&u64::try_from(payload.len())?.to_be_bytes());
    }
    frame.extend_from_slice(&MASK);
    frame.extend(payload.iter().zip(MASK.iter().cycle()).map(|(b, k)| b ^ k));
    Ok(frame)
}

fn decode(bytes: &[u8]) -> TestResult<Option<(Received, usize)>> {
    let [first, second, ..] = *bytes else {
        return Ok(None);
    };
    if second & 0x80 != 0 || first & 0x80 == 0 {
        return Err("server frames are unmasked and unfragmented".into());
    }
    let (length, offset): (usize, usize) = match second & 0x7F {
        0x7E => match bytes.get(2..4) {
            Some(&[high, low]) => (usize::from(u16::from_be_bytes([high, low])), 4),
            _ => return Ok(None),
        },
        0x7F => match bytes.get(2..10) {
            Some(long) => (usize::try_from(u64::from_be_bytes(long.try_into()?))?, 10),
            None => return Ok(None),
        },
        short => (usize::from(short), 2),
    };
    let end = offset.checked_add(length).ok_or("frame length overflows")?;
    let Some(payload) = bytes.get(offset..end) else {
        return Ok(None);
    };
    let received = match first & 0x0F {
        0x2 => Received::Binary(payload.to_vec()),
        0xA => Received::Pong(payload.to_vec()),
        0x8 => match payload {
            [high, low, reason @ ..] => Received::Close(
                Some(u16::from_be_bytes([*high, *low])),
                String::from_utf8(reason.to_vec())?,
            ),
            _ => Received::Close(None, String::new()),
        },
        other => return Err(format!("unexpected server opcode {other}").into()),
    };
    Ok(Some((received, end)))
}
