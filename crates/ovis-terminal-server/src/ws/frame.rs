//! RFC 6455 frames (section 5): the server's decoder for client frames and its encoder.
//!
//! Client frames must be masked and carry no extension bits. Binary data frames, whole
//! or fragmented, are handed on payload by payload, because the connection reads one
//! byte stream and message boundaries carry no meaning inbound. Text frames are refused.
//! Server frames are unmasked and never fragmented.

use super::WsError;
use super::error::FrameDefect;

const FIN: u8 = 0x80;
const RESERVED: u8 = 0x70;
const MASKED: u8 = 0x80;
const MAX_CONTROL_PAYLOAD: u8 = 125;

/// Frame opcodes this server sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Opcode {
    Binary,
    Close,
    Pong,
}

impl Opcode {
    const fn bits(self) -> u8 {
        match self {
            Self::Binary => 0x2,
            Self::Close => 0x8,
            Self::Pong => 0xA,
        }
    }
}

/// One decoded client frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Inbound {
    /// Payload of a binary data frame or one of its continuations.
    Data(Vec<u8>),
    /// A ping; its payload is echoed in the pong.
    Ping(Vec<u8>),
    /// An unsolicited or answering pong.
    Pong,
    /// The peer closes, with its status code if it sent one.
    Close(Option<u16>),
}

/// Decodes client frames from bytes that arrive split anywhere.
#[derive(Debug)]
pub(super) struct FrameDecoder {
    buffer: Vec<u8>,
    fragmented: bool,
    max_payload: usize,
}

impl FrameDecoder {
    /// A decoder refusing frames whose payload exceeds `max_payload` bytes.
    pub(super) const fn new(max_payload: usize) -> Self {
        Self {
            buffer: Vec::new(),
            fragmented: false,
            max_payload,
        }
    }

    pub(super) fn feed(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// The next whole frame, or `None` until more bytes arrive.
    ///
    /// # Errors
    ///
    /// [`WsError::TextMessage`], [`WsError::MessageTooLarge`] or [`WsError::Protocol`].
    pub(super) fn next(&mut self) -> Result<Option<Inbound>, WsError> {
        let Some(header) = Header::parse(&self.buffer)? else {
            return Ok(None);
        };
        if header.length > self.max_payload {
            return Err(WsError::MessageTooLarge {
                length: header.length,
            });
        }
        let end = header.offset.saturating_add(header.length);
        let Some(masked) = self.buffer.get(header.offset..end) else {
            return Ok(None);
        };
        let payload = masked
            .iter()
            .zip(header.mask.iter().cycle())
            .map(|(byte, key)| byte ^ key)
            .collect();
        self.buffer.drain(..end);
        self.decide(header, payload).map(Some)
    }

    fn decide(&mut self, header: Header, payload: Vec<u8>) -> Result<Inbound, WsError> {
        let defect = |defect| Err(WsError::Protocol(defect));
        match (header.opcode, self.fragmented) {
            (0x1, _) => Err(WsError::TextMessage),
            (0x0, false) => defect(FrameDefect::UnexpectedContinuation),
            (0x2, true) => defect(FrameDefect::InterleavedMessage),
            (0x0 | 0x2, _) => {
                self.fragmented = !header.fin;
                Ok(Inbound::Data(payload))
            }
            (0x8, _) => match payload.as_slice() {
                [] => Ok(Inbound::Close(None)),
                [high, low, ..] => Ok(Inbound::Close(Some(u16::from_be_bytes([*high, *low])))),
                [_] => defect(FrameDefect::CloseLength),
            },
            (0x9, _) => Ok(Inbound::Ping(payload)),
            (0xA, _) => Ok(Inbound::Pong),
            _ => defect(FrameDefect::UnknownOpcode),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Header {
    fin: bool,
    opcode: u8,
    mask: [u8; 4],
    offset: usize,
    length: usize,
}

impl Header {
    fn parse(buffer: &[u8]) -> Result<Option<Self>, WsError> {
        let [first, second, ..] = *buffer else {
            return Ok(None);
        };
        let (fin, opcode) = (first & FIN != 0, first & 0x0F);
        let defect = |defect| Err(WsError::Protocol(defect));
        if first & RESERVED != 0 {
            return defect(FrameDefect::ReservedBits);
        }
        if second & MASKED == 0 {
            return defect(FrameDefect::Unmasked);
        }
        let (length, offset): (u64, usize) = match second & 0x7F {
            126 => match buffer.get(2..4) {
                Some(&[high, low]) => (u64::from(u16::from_be_bytes([high, low])), 4),
                _ => return Ok(None),
            },
            127 => match buffer.get(2..10).map(<[u8; 8]>::try_from) {
                Some(Ok(bytes)) => (u64::from_be_bytes(bytes), 10),
                _ => return Ok(None),
            },
            short => (u64::from(short), 2),
        };
        let control = opcode & 0x8 != 0;
        if control && (!fin || length > u64::from(MAX_CONTROL_PAYLOAD)) {
            return defect(FrameDefect::Control);
        }
        let Some(&[a, b, c, d]) = buffer.get(offset..offset.saturating_add(4)) else {
            return Ok(None);
        };
        Ok(Some(Self {
            fin,
            opcode,
            mask: [a, b, c, d],
            offset: offset.saturating_add(4),
            length: usize::try_from(length).unwrap_or(usize::MAX),
        }))
    }
}

/// One unmasked, unfragmented server frame.
pub(super) fn encode(opcode: Opcode, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(payload.len().saturating_add(10));
    frame.push(FIN | opcode.bits());
    let length = payload.len();
    if let Some(short) = u8::try_from(length).ok().filter(|short| *short < 126) {
        frame.push(short);
    } else if let Ok(medium) = u16::try_from(length) {
        frame.push(126);
        frame.extend_from_slice(&medium.to_be_bytes());
    } else {
        frame.push(127);
        let long = u64::try_from(length).unwrap_or(u64::MAX);
        frame.extend_from_slice(&long.to_be_bytes());
    }
    frame.extend_from_slice(payload);
    frame
}

/// A close frame with `code` and a machine-readable reason token.
pub(super) fn close(code: u16, reason: &str) -> Vec<u8> {
    let reason = reason.as_bytes();
    let room = usize::from(MAX_CONTROL_PAYLOAD).saturating_sub(2);
    let reason = reason.get(..room.min(reason.len())).unwrap_or_default();
    encode(Opcode::Close, &[&code.to_be_bytes()[..], reason].concat())
}

#[cfg(test)]
mod tests;
