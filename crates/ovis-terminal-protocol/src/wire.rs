//! Incremental channel framing and bounded decoding.

mod connection;
mod decoder;
mod error;
mod frame;
mod logical;
mod offset;
#[cfg(test)]
mod properties;

pub use connection::{ConnectionDecoder, FeedOutcome, WireMessage};
pub use decoder::FrameDecoder;
pub use error::FrameError;
pub use frame::{Channel, Frame, FrameKind};
pub use logical::{ChunkMetadata, LogicalChunk, LogicalChunks, LogicalResult};
pub use offset::{Continuation, LogicalOffset, LogicalTotal, WireOffset};

/// Maximum payload in one framed message and one NDJSON control record.
pub const MAX_FRAME_PAYLOAD: usize = 16 * 1024 * 1024;
/// Number of bytes in the little-endian channel/kind/length frame header.
pub const FRAME_HEADER_LEN: usize = 9;
/// Maximum decoded messages retained by one connection.
pub const MAX_READY_RECORDS: usize = 64;
/// Maximum payload bytes retained in one connection's ready queue.
pub const MAX_READY_BYTES: usize = 32 * 1024 * 1024;
