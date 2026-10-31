//! Observation of a transcript file: identity, length and content digest.
//!
//! Comparing a fresh read with the recorded observation tells an append from a
//! truncation, a replaced file and an in-place rewrite that keeps the size.

use std::fs::{File, Metadata};
use std::io::Read as _;

use ovis_terminal_protocol::domain::session_binding::HistoryChange;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::{BindingError, TranscriptPath};

/// Largest transcript the runtime reads, in bytes.
pub(super) const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// The file a path named when it was observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct FileIdentity {
    volume: u64,
    file: u64,
}

#[cfg(unix)]
#[expect(
    clippy::unnecessary_wraps,
    reason = "shares the fallible signature of the creation-time identity used without Unix"
)]
fn identity(metadata: &Metadata) -> std::io::Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt as _;
    Ok(FileIdentity {
        volume: metadata.dev(),
        file: metadata.ino(),
    })
}

/// Without a stable file index, creation time distinguishes a replacement file.
#[cfg(not(unix))]
fn identity(metadata: &Metadata) -> std::io::Result<FileIdentity> {
    let created = metadata
        .created()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(std::io::Error::other)?;
    Ok(FileIdentity {
        volume: created.as_secs(),
        file: u64::from(created.subsec_nanos()),
    })
}

/// What was recorded about a transcript at one revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Observation {
    identity: FileIdentity,
    len: u64,
    /// Lowercase hex SHA-256 of the first `len` bytes.
    digest: String,
}

/// One read of a transcript and the observation it yields.
#[derive(Debug)]
pub(super) struct Content {
    pub observation: Observation,
    pub bytes: Vec<u8>,
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect()
}

/// Reads `path` once, from one open handle, so identity and bytes describe one file.
pub(super) fn read(path: &TranscriptPath) -> Result<Content, BindingError> {
    let missing = |source| BindingError::SourceMissing { source };
    let file = File::open(path.as_path()).map_err(missing)?;
    let identity = identity(&file.metadata().map_err(missing)?).map_err(missing)?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(missing)?;
    let len = u64::try_from(bytes.len()).map_err(|_| BindingError::TooLarge)?;
    if len > MAX_BYTES {
        return Err(BindingError::TooLarge);
    }
    Ok(Content {
        observation: Observation {
            identity,
            len,
            digest: hex(&bytes),
        },
        bytes,
    })
}

impl Content {
    /// What differs from `recorded`; `None` when the transcript is unchanged.
    pub(super) fn change_since(&self, recorded: &Observation) -> Option<HistoryChange> {
        let now = &self.observation;
        if now.identity != recorded.identity {
            return Some(HistoryChange::Replace);
        }
        if now.len < recorded.len {
            return Some(HistoryChange::Truncate);
        }
        let prefix = usize::try_from(recorded.len)
            .ok()
            .and_then(|len| self.bytes.get(..len));
        if prefix.is_none_or(|prefix| hex(prefix) != recorded.digest) {
            return Some(HistoryChange::Rewrite);
        }
        (now.len > recorded.len).then_some(HistoryChange::Append)
    }
}
