//! Versioned holder frame header.

use super::{HOLDER_PROTOCOL_GENERATION, MAX_FRAME_PAYLOAD, MessageKind};

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

/// Header parse or frame-bound failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum FrameError {
    /// Input did not contain a complete header with the expected magic/flags.
    MalformedHeader,
    /// Peer selected a different holder protocol generation.
    UnsupportedGeneration(u16),
    /// Message tag is not assigned in this protocol generation.
    UnknownMessageKind(u8),
    /// Payload exceeds the version-one frame limit.
    FrameTooLarge(u32),
    /// Payload length cannot be represented by the wire format.
    LengthOverflow,
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
