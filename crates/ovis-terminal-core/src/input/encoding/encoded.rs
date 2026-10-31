//! One encoded queue entry and the evidence its receipt carries.

use ovis_terminal_protocol::schema::scalar::{ByteCount, Revision};
use ovis_terminal_vt::ModeRevision;
use serde::{Deserialize, Serialize};

use super::error::EncodingError;

/// Which input path produced an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InputKind {
    /// Literal text with its submission (`input.send`).
    Text,
    /// Pasted text (`input.paste`).
    Paste,
    /// Named keys (`input.keys`).
    Keys,
    /// Mouse reports.
    Mouse,
    /// Authorized raw bytes (`input.raw`).
    Raw,
}

/// What an entry was encoded against and how its bytes divide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EncodingEvidence {
    /// Input path.
    pub kind: InputKind,
    /// Mode revision the bytes were decided against.
    pub mode_revision: ModeRevision,
    /// Whether the literal is wrapped in bracketed-paste markers.
    pub bracketed: bool,
    /// Leading bytes holding the literal, markers included.
    pub literal_bytes: ByteCount,
    /// Trailing bytes holding the submission.
    pub submit_bytes: ByteCount,
}

/// The exact bytes of one queue entry; nothing is added, dropped or reordered later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoded {
    bytes: Vec<u8>,
    evidence: EncodingEvidence,
}

fn count(bytes: &[u8]) -> Result<ByteCount, EncodingError> {
    u64::try_from(bytes.len())
        .map(ByteCount::new)
        .map_err(|_| EncodingError::Oversized)
}

/// The fixed facts of an entry before its bytes are known.
#[derive(Debug, Clone, Copy)]
pub(super) struct Entry {
    pub(super) kind: InputKind,
    pub(super) mode_revision: ModeRevision,
    pub(super) bracketed: bool,
}

impl Entry {
    /// Joins the literal and the submission into one entry.
    pub(super) fn finish(
        self,
        mut literal: Vec<u8>,
        submit: &[u8],
    ) -> Result<Encoded, EncodingError> {
        if literal.is_empty() && submit.is_empty() {
            return Err(EncodingError::Empty);
        }
        let evidence = EncodingEvidence {
            kind: self.kind,
            mode_revision: self.mode_revision,
            bracketed: self.bracketed,
            literal_bytes: count(&literal)?,
            submit_bytes: count(submit)?,
        };
        literal.extend_from_slice(submit);
        count(&literal)?;
        Ok(Encoded {
            bytes: literal,
            evidence,
        })
    }
}

impl Encoded {
    /// The bytes to write, in order.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes the entry into its bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// What the bytes were encoded against.
    pub const fn evidence(&self) -> &EncodingEvidence {
        &self.evidence
    }

    /// The mode revision as the admission receipt publishes it.
    pub const fn receipt_revision(&self) -> Revision {
        Revision::new(self.evidence.mode_revision.get())
    }

    /// Confirms the terminal is still at the revision the bytes were encoded against.
    ///
    /// # Errors
    ///
    /// Returns `mode_mismatch` when the modes moved; encode again instead of writing.
    pub fn ensure_current(&self, current: ModeRevision) -> Result<(), EncodingError> {
        if current == self.evidence.mode_revision {
            Ok(())
        } else {
            Err(EncodingError::ModeChanged {
                encoded: self.evidence.mode_revision,
                current,
            })
        }
    }
}
