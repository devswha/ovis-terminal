//! Incremental channel framing and bounded decoding.

mod connection;
mod decoder;
mod error;
mod frame;
mod logical;

pub use connection::{ConnectionDecoder, WireMessage};
pub use decoder::FrameDecoder;
pub use error::FrameError;
pub use frame::{Channel, Frame, FrameKind};
pub use logical::{ChunkMetadata, LogicalChunk, LogicalChunks, LogicalResult};

/// Maximum payload in one framed message and one NDJSON control record.
pub const MAX_FRAME_PAYLOAD: usize = 16 * 1024 * 1024;
/// Number of bytes in the little-endian channel/kind/length frame header.
pub const FRAME_HEADER_LEN: usize = 9;
