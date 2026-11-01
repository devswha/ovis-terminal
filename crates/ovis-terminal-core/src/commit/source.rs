//! Which producer committed a record, and the local position it was given.

use ovis_terminal_protocol::schema::scalar::EventSeq;
use serde::{Deserialize, Serialize};

use crate::events::{Cursor, RemoteOrigin};
use crate::registry::TerminalId;

/// The producer of one commit; all of them share one local sequence domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum CommitSource {
    /// A terminal actor, after its own admission barrier.
    Terminal {
        /// The terminal whose actor committed.
        terminal_id: TerminalId,
    },
    /// The registry of identities, layout and agents.
    Registry,
    /// A remote machine's event, mirrored at its upstream position.
    Remote {
        /// Upstream machine, lifetime and sequence; never the local sequence.
        origin: RemoteOrigin,
    },
}

/// A durably committed record: its local cursor and the source that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Committed {
    /// Local position allocated by the serialized durable commit.
    pub cursor: Cursor,
    /// Producer of the record.
    pub source: CommitSource,
}

impl Committed {
    /// The local sequence; a remote record's upstream sequence is in [`Self::remote`].
    pub const fn local_seq(&self) -> EventSeq {
        self.cursor.seq
    }

    /// The upstream position of a mirrored record.
    pub const fn remote(&self) -> Option<&RemoteOrigin> {
        match &self.source {
            CommitSource::Remote { origin } => Some(origin),
            CommitSource::Terminal { .. } | CommitSource::Registry => None,
        }
    }
}
