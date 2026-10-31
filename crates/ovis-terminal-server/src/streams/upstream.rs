//! Upstream frames on framed stream channels: download-only, CREDIT and CLOSE.
//!
//! Each stream is bound to a channel number that is never reused on its connection.
//! CREDIT carries the eight-byte little-endian cumulative filtered-stream offset; a
//! CREDIT for a retired channel is refused, never applied to a newer stream. DATA from
//! the client is a direction violation: nothing reaches the program.

use std::collections::{HashMap, HashSet};

use ovis_terminal_protocol::schema::ids::StreamId;
use ovis_terminal_protocol::wire::{Channel, Frame, FrameKind};

use super::credit::CreditRefusal;
use super::error::StreamError;
use super::offset::StreamOffset;

/// What an upstream frame asks of a stream.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Upstream {
    /// The client consumed the stream up to `acknowledged`.
    Credit {
        /// Stream bound to the channel.
        stream: StreamId,
        /// Cumulative filtered-stream offset consumed.
        acknowledged: StreamOffset,
    },
    /// The client closed the stream.
    Close {
        /// Stream bound to the channel.
        stream: StreamId,
    },
}

/// Stream channels of one framed connection.
#[derive(Debug)]
pub struct StreamChannels {
    next: u32,
    live: HashMap<u32, StreamId>,
    retired: HashSet<u32>,
}

impl Default for StreamChannels {
    fn default() -> Self {
        Self {
            next: 1,
            live: HashMap::new(),
            retired: HashSet::new(),
        }
    }
}

impl StreamChannels {
    /// Binds `stream` to the next unused channel.
    ///
    /// # Errors
    ///
    /// Returns `stream_limit` once every channel number of the connection was used.
    pub fn bind(&mut self, stream: StreamId) -> Result<Channel, StreamError> {
        let channel = self.next;
        self.next = channel.checked_add(1).ok_or(StreamError::StreamLimit {
            limit: usize::try_from(u32::MAX).unwrap_or(usize::MAX),
        })?;
        self.live.insert(channel, stream);
        Ok(Channel::new(channel))
    }

    /// Retires `channel`; its number is never bound again on this connection.
    pub fn retire(&mut self, channel: Channel) {
        if self.live.remove(&channel.value()).is_some() {
            self.retired.insert(channel.value());
        }
    }

    /// Interprets one client frame on a stream channel.
    ///
    /// # Errors
    ///
    /// Returns `direction_violation` for DATA, `invalid_credit` for a malformed CREDIT
    /// or one on a retired channel, and `stream_not_found` for a channel never bound.
    pub fn upstream(&self, frame: &Frame) -> Result<Upstream, StreamError> {
        let channel = frame.channel.value();
        if frame.kind == FrameKind::Data {
            return Err(StreamError::DirectionViolation { channel });
        }
        let Some(stream) = self.live.get(&channel).cloned() else {
            return Err(if self.retired.contains(&channel) {
                StreamError::InvalidCredit(CreditRefusal::Closed)
            } else {
                StreamError::StreamNotFound
            });
        };
        match frame.kind {
            FrameKind::Credit => {
                let bytes: [u8; 8] = frame.payload.as_slice().try_into().map_err(|_| {
                    StreamError::MalformedCredit {
                        len: frame.payload.len(),
                    }
                })?;
                Ok(Upstream::Credit {
                    stream,
                    acknowledged: StreamOffset::new(u64::from_le_bytes(bytes)),
                })
            }
            _ => Ok(Upstream::Close { stream }),
        }
    }
}

#[cfg(test)]
mod tests {
    use ovis_terminal_protocol::schema::ids::StreamId;
    use ovis_terminal_protocol::wire::{Frame, FrameKind};

    use super::{StreamChannels, Upstream};
    use crate::streams::offset::StreamOffset;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn credit_decodes_u64_max_exactly_and_retired_channels_are_never_reused() -> TestResult {
        let mut channels = StreamChannels::default();
        let first = channels.bind(StreamId::new("stream:a".to_owned()))?;
        let credit = Frame::new(first, FrameKind::Credit, u64::MAX.to_le_bytes().to_vec())?;
        assert_eq!(
            channels.upstream(&credit)?,
            Upstream::Credit {
                stream: StreamId::new("stream:a".to_owned()),
                acknowledged: StreamOffset::new(u64::MAX),
            }
        );
        channels.retire(first);
        let second = channels.bind(StreamId::new("stream:b".to_owned()))?;
        assert_ne!(second, first);
        let stale = channels.upstream(&credit).map_err(|error| error.kind());
        assert_eq!(stale, Err("invalid_credit"));
        let data = Frame::new(second, FrameKind::Data, b"rm -rf /\r".to_vec())?;
        let violation = channels.upstream(&data).map_err(|error| error.kind());
        assert_eq!(violation, Err("direction_violation"));
        let short = Frame::new(second, FrameKind::Credit, vec![1, 2, 3])?;
        assert_eq!(
            channels.upstream(&short).map_err(|e| e.kind()),
            Err("invalid_credit")
        );
        Ok(())
    }
}
