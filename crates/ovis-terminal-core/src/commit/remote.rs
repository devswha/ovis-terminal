//! The durable upstream high-water mark of each mirrored remote lifetime.
//!
//! It is read and advanced inside the same transaction as the mirrored record, so a
//! replayed or reordered upstream position is refused before any local sequence exists.

use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_store::{CasWrite, Payload, ResourceKey, ResourceLookup, Snapshot, Version};

use super::CommitError;
use crate::events::RemoteOrigin;

const KIND: &str = "commit.remote";

fn key(origin: &RemoteOrigin) -> ResourceKey {
    ResourceKey::new(KIND, format!("{}/{}", origin.machine_id, origin.runtime_id))
}

/// The CAS write that advances `origin`'s lifetime to its sequence.
///
/// # Errors
///
/// Returns `remote_replayed` when the upstream sequence is not after the committed
/// high-water mark, or `invalid_event` when the stored mark is unreadable.
pub(super) fn advance(snapshot: &Snapshot, origin: &RemoteOrigin) -> Result<CasWrite, CommitError> {
    let key = key(origin);
    let (expected, last) = match snapshot.resource(&key).map_err(CommitError::from_store)? {
        ResourceLookup::Live { version, value, .. } => {
            let last: EventSeq = serde_json::from_slice(value.as_bytes())
                .map_err(|source| CommitError::RemoteMark { source })?;
            (version, Some(last))
        }
        ResourceLookup::Tombstone { version, .. } => (version, None),
        ResourceLookup::Absent => (Version::ABSENT, None),
        _ => return Err(CommitError::RemoteMarkUnknown),
    };
    let upstream = EventSeq::new(origin.seq.get());
    if let Some(last) = last.filter(|last| upstream <= *last) {
        return Err(CommitError::RemoteReplayed { upstream, last });
    }
    let mark =
        serde_json::to_vec(&upstream).map_err(|source| CommitError::RemoteMark { source })?;
    Ok(CasWrite::new(key, expected, Some(Payload::from(mark))))
}
