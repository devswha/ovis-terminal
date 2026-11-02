//! The `stream_data` step: what a client of one attach stream received from an offset on.

use serde::Deserialize;

use crate::ids::ConnName;

/// Collect one stream's `attach.data` bytes from stream offset `from` (a decimal or a
/// `$value:` label) until they contain `until`; the bytes are kept as an assertion named
/// `stream[<id>]/data_b64`, so a test can rebuild the screen a client would show.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamData {
    pub(crate) id: String,
    pub(crate) conn: ConnName,
    /// Stream id, usually `$value:<captured stream>`.
    pub(crate) stream: String,
    pub(crate) from: String,
    pub(crate) until: String,
}
