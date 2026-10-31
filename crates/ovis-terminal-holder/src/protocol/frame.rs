//! Versioned holder frame header.

use thiserror::Error;

use super::{HOLDER_PROTOCOL_GENERATION, MAX_FRAME_PAYLOAD, Message, MessageKind};

/// Fixed header size: magic, generation, kind, flags, and payload length.
const HEADER_LEN: usize = 12;
const MAGIC: [u8; 4] = *b"OVHP";

/// Parsed holder frame header; the payload follows as a bounded byte slice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct FrameHeader {
    /// Independently versioned holder protocol generation.
    pub generation: u16,
    /// Control message discriminator.
    pub kind: MessageKind,
    /// Payload length in bytes.
    pub payload_len: u32,
}

/// Incremental holder frame decoder with a bounded per-frame buffer.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct FrameDecoder {
    buffer: Vec<u8>,
    expected_frame_len: Option<usize>,
}

impl FrameDecoder {
    /// Adds a byte fragment and returns each complete decoded message.
    ///
    /// # Errors
    ///
    /// Returns a typed error for malformed headers, unsupported generations,
    /// oversized frames, malformed payloads, or message-kind mismatches.
    pub fn push(&mut self, mut bytes: &[u8]) -> Result<Vec<Message>, FrameError> {
        let mut messages = Vec::new();
        while !bytes.is_empty() {
            let needed = self.expected_frame_len.map_or_else(
                || HEADER_LEN.saturating_sub(self.buffer.len()),
                |frame_len| frame_len.saturating_sub(self.buffer.len()),
            );
            let take = bytes.len().min(needed);
            let (fragment, remaining) = bytes.split_at(take);
            self.buffer.extend_from_slice(fragment);
            bytes = remaining;

            if self.expected_frame_len.is_none() && self.buffer.len() == HEADER_LEN {
                let (header, header_len) = match parse_header(&self.buffer) {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        self.buffer.clear();
                        return Err(error);
                    }
                };
                let Ok(payload_len) = usize::try_from(header.payload_len) else {
                    self.buffer.clear();
                    return Err(FrameError::LengthOverflow);
                };
                let Some(expected_frame_len) = header_len.checked_add(payload_len) else {
                    self.buffer.clear();
                    return Err(FrameError::LengthOverflow);
                };
                self.expected_frame_len = Some(expected_frame_len);
            }
            if self
                .expected_frame_len
                .is_some_and(|frame_len| self.buffer.len() == frame_len)
            {
                let frame = std::mem::take(&mut self.buffer);
                self.expected_frame_len = None;
                messages.push(decode_message(&frame)?);
            }
        }
        Ok(messages)
    }

    /// Reports whether the stream ended between frames.
    ///
    /// # Errors
    ///
    /// Returns [`FrameError::TruncatedFrame`] when buffered bytes do not form
    /// one complete frame.
    pub const fn finish(&self) -> Result<(), FrameError> {
        if self.buffer.is_empty() && self.expected_frame_len.is_none() {
            Ok(())
        } else {
            Err(FrameError::TruncatedFrame)
        }
    }
}

impl FrameHeader {
    /// Encodes the header using little-endian integer fields.
    ///
    /// # Errors
    ///
    /// Returns [`FrameError::FrameTooLarge`] when the payload exceeds the
    /// version-one maximum.
    pub fn encode(kind: MessageKind, payload_len: usize) -> Result<[u8; HEADER_LEN], FrameError> {
        let length = u32::try_from(payload_len).map_err(|_| FrameError::LengthOverflow)?;
        if payload_len > MAX_FRAME_PAYLOAD {
            return Err(FrameError::FrameTooLarge(length));
        }
        let mut bytes = [0; HEADER_LEN];
        bytes[..4].copy_from_slice(&MAGIC);
        bytes[4..6].copy_from_slice(&HOLDER_PROTOCOL_GENERATION.to_le_bytes());
        bytes[6] = kind.wire_tag();
        bytes[7] = 0;
        bytes[8..12].copy_from_slice(&length.to_le_bytes());
        Ok(bytes)
    }
}

/// Encodes one typed message as a versioned holder frame.
///
/// # Errors
///
/// Returns a typed error when serialization fails or the payload exceeds the
/// version-one frame limit.
pub fn encode_message(message: &Message) -> Result<Vec<u8>, FrameError> {
    let payload = super::payload::encode(message)?;
    let header = FrameHeader::encode(message.kind(), payload.len())?;
    let frame_capacity = HEADER_LEN
        .checked_add(payload.len())
        .ok_or(FrameError::LengthOverflow)?;
    let mut frame = Vec::with_capacity(frame_capacity);
    frame.extend_from_slice(&header);
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Decodes exactly one complete holder frame.
///
/// # Errors
///
/// Returns a typed error for a truncated or malformed frame, unsupported
/// generation, malformed payload, trailing bytes, or a payload/header tag
/// mismatch.
pub fn decode_message(frame: &[u8]) -> Result<Message, FrameError> {
    if frame.len() < HEADER_LEN {
        return Err(FrameError::TruncatedFrame);
    }
    let (header, header_len) = parse_header(frame)?;
    let payload_len =
        usize::try_from(header.payload_len).map_err(|_| FrameError::LengthOverflow)?;
    let expected_len = header_len
        .checked_add(payload_len)
        .ok_or(FrameError::LengthOverflow)?;
    if frame.len() < expected_len {
        return Err(FrameError::TruncatedFrame);
    }
    if frame.len() > expected_len {
        return Err(FrameError::TrailingBytes);
    }
    let payload = frame
        .get(header_len..expected_len)
        .ok_or(FrameError::TruncatedFrame)?;
    let message = super::payload::decode(payload)?;
    if message.kind() != header.kind {
        return Err(FrameError::MessageKindMismatch);
    }
    Ok(message)
}

/// Header parse or frame-bound failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FrameError {
    /// Input did not contain a complete header with the expected magic/flags.
    #[error("holder frame header is malformed")]
    MalformedHeader,
    /// Peer selected a different holder protocol generation.
    #[error("holder frame protocol generation is unsupported")]
    UnsupportedGeneration(u16),
    /// Message tag is not assigned in this protocol generation.
    #[error("holder frame message kind is unknown")]
    UnknownMessageKind(u8),
    /// Payload exceeds the version-one frame limit.
    #[error("holder frame payload exceeds the configured limit")]
    FrameTooLarge(u32),
    /// Payload length cannot be represented by the wire format.
    #[error("holder frame payload length cannot be represented")]
    LengthOverflow,
    /// Frame ended before the header-declared payload was complete.
    #[error("holder frame is truncated")]
    TruncatedFrame,
    /// Frame contains bytes beyond its declared payload.
    #[error("holder frame has trailing bytes")]
    TrailingBytes,
    /// Header tag and decoded message tag disagree.
    #[error("holder frame tag does not match its decoded message")]
    MessageKindMismatch,
    /// Payload bytes are not a valid version-one message.
    #[error("holder frame payload is malformed")]
    MalformedPayload {
        /// Serialization failure while decoding the payload.
        #[source]
        source: serde_json::Error,
    },
    /// Typed message could not be serialized into the frame payload.
    #[error("holder message could not be serialized")]
    PayloadSerialization {
        /// Serialization failure while encoding the payload.
        #[source]
        source: serde_json::Error,
    },
}

impl FrameError {
    /// Returns the stable machine-readable error kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::MalformedHeader => "malformed_frame",
            Self::UnsupportedGeneration(_) => "unsupported_generation",
            Self::UnknownMessageKind(_) => "unknown_message_kind",
            Self::FrameTooLarge(_) => "frame_too_large",
            Self::LengthOverflow => "frame_length_overflow",
            Self::TruncatedFrame => "truncated_frame",
            Self::TrailingBytes => "trailing_frame_bytes",
            Self::MessageKindMismatch => "message_kind_mismatch",
            Self::MalformedPayload { .. } => "malformed_payload",
            Self::PayloadSerialization { .. } => "payload_serialization_failed",
        }
    }

    /// Returns a static recovery hint suitable for a client or operator.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::MalformedHeader => {
                "Send a complete v1 holder frame with the OVHP magic and zero flags."
            }
            Self::UnsupportedGeneration(_) => {
                "Upgrade or select a peer that supports the holder protocol generation."
            }
            Self::UnknownMessageKind(_) => {
                "Use a message kind assigned by the negotiated holder generation."
            }
            Self::FrameTooLarge(_) => "Split the payload into frames no larger than 16 MiB.",
            Self::LengthOverflow => {
                "Encode a payload length representable by the holder frame format."
            }
            Self::TruncatedFrame => {
                "Continue reading until the declared frame payload is complete."
            }
            Self::TrailingBytes => {
                "Decode one frame at a time or pass the remainder to the next decode."
            }
            Self::MessageKindMismatch => {
                "Encode the payload using the message kind in the frame header."
            }
            Self::MalformedPayload { .. } => "Send a valid v1 holder message payload.",
            Self::PayloadSerialization { .. } => {
                "Review the holder message payload before retrying."
            }
        }
    }
}

/// Parses and validates a complete fixed-size header without ownership effects.
///
/// # Errors
///
/// Returns [`FrameError`] for malformed, unsupported, unknown, or oversized input.
pub fn parse_header(bytes: &[u8]) -> Result<(FrameHeader, usize), FrameError> {
    let header = bytes.get(..HEADER_LEN).ok_or(FrameError::MalformedHeader)?;
    if header.get(..4) != Some(MAGIC.as_slice()) || header.get(7) != Some(&0) {
        return Err(FrameError::MalformedHeader);
    }
    let generation_bytes: [u8; 2] = header
        .get(4..6)
        .ok_or(FrameError::MalformedHeader)?
        .try_into()
        .map_err(|_| FrameError::MalformedHeader)?;
    let generation = u16::from_le_bytes(generation_bytes);
    if generation != HOLDER_PROTOCOL_GENERATION {
        return Err(FrameError::UnsupportedGeneration(generation));
    }
    let tag = *header.get(6).ok_or(FrameError::MalformedHeader)?;
    let kind = MessageKind::from_wire_tag(tag).map_err(FrameError::UnknownMessageKind)?;
    let length_bytes: [u8; 4] = header
        .get(8..12)
        .ok_or(FrameError::MalformedHeader)?
        .try_into()
        .map_err(|_| FrameError::MalformedHeader)?;
    let payload_len = u32::from_le_bytes(length_bytes);
    if usize::try_from(payload_len).map_err(|_| FrameError::LengthOverflow)? > MAX_FRAME_PAYLOAD {
        return Err(FrameError::FrameTooLarge(payload_len));
    }
    Ok((
        FrameHeader {
            generation,
            kind,
            payload_len,
        },
        HEADER_LEN,
    ))
}
