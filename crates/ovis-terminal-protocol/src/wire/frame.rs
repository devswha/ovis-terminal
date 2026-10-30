use super::{FRAME_HEADER_LEN, FrameError, MAX_FRAME_PAYLOAD};

/// A channel identifier, with channel zero reserved for JSON-RPC control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Channel(u32);

impl Channel {
    /// JSON-RPC control channel.
    pub const CONTROL: Self = Self(0);

    /// Creates a channel identifier.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the encoded channel identifier.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

/// A framed protocol message kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[repr(u8)]
pub enum FrameKind {
    /// Carries JSON-RPC on channel zero or terminal bytes on a stream channel.
    Data = 0,
    /// Carries a cumulative eight-byte little-endian delivery offset.
    Credit = 1,
    /// Closes a channel with a typed control payload.
    Close = 2,
}

impl TryFrom<u8> for FrameKind {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Data),
            1 => Ok(Self::Credit),
            2 => Ok(Self::Close),
            other => Err(other),
        }
    }
}

impl FrameKind {
    const fn wire_value(self) -> u8 {
        match self {
            Self::Data => 0,
            Self::Credit => 1,
            Self::Close => 2,
        }
    }
}

/// A single bounded wire frame.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Frame {
    /// Channel carrying the message.
    pub channel: Channel,
    /// Message kind.
    pub kind: FrameKind,
    /// Opaque bytes belonging to this channel and kind.
    pub payload: Vec<u8>,
}

impl Frame {
    /// Creates a frame after enforcing the payload bound.
    ///
    /// # Errors
    /// Returns [`FrameError::FrameTooLarge`] when the payload exceeds 16 MiB.
    pub fn new(channel: Channel, kind: FrameKind, payload: Vec<u8>) -> Result<Self, FrameError> {
        if payload.len() > MAX_FRAME_PAYLOAD {
            return Err(FrameError::FrameTooLarge {
                length: payload.len(),
                limit: MAX_FRAME_PAYLOAD,
                offset: 0,
            });
        }
        Ok(Self {
            channel,
            kind,
            payload,
        })
    }

    /// Encodes the fixed nine-byte header followed by the payload.
    ///
    /// # Errors
    /// Returns [`FrameError::FrameTooLarge`] when the payload exceeds 16 MiB.
    pub fn encode(&self) -> Result<Vec<u8>, FrameError> {
        if self.payload.len() > MAX_FRAME_PAYLOAD {
            return Err(FrameError::FrameTooLarge {
                length: self.payload.len(),
                limit: MAX_FRAME_PAYLOAD,
                offset: 0,
            });
        }
        let length = u32::try_from(self.payload.len()).map_err(|_| FrameError::FrameTooLarge {
            length: self.payload.len(),
            limit: MAX_FRAME_PAYLOAD,
            offset: 0,
        })?;
        let mut encoded =
            Vec::with_capacity(FRAME_HEADER_LEN.checked_add(self.payload.len()).ok_or(
                FrameError::OffsetOverflow {
                    offset: 0,
                    amount: self.payload.len(),
                },
            )?);
        encoded.extend_from_slice(&self.channel.0.to_le_bytes());
        encoded.push(self.kind.wire_value());
        encoded.extend_from_slice(&length.to_le_bytes());
        encoded.extend_from_slice(&self.payload);
        Ok(encoded)
    }
}
