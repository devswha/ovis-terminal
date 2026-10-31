//! Typed session references: a transcript path inside the authorized store that
//! `session.source` names, or an opaque provider session key. Neither is ever chosen by
//! directory or recency.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ovis_terminal_protocol::schema::records::SessionRef;
use serde::{Deserialize, Serialize};

use super::BindingError;

/// Largest source label or session key, in bytes.
const NAME_LIMIT: usize = 256;

/// A canonical transcript file inside one of the authorized stores.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TranscriptPath(PathBuf);

impl TranscriptPath {
    /// The canonical path.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// A provider session identifier with no readable store behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionKey(String);

/// What a session reference names, validated once at the boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SourceRef {
    /// `kind: "path"`: a transcript file the runtime can observe.
    Transcript(TranscriptPath),
    /// `kind: "id"`: an opaque provider session.
    Session(SessionKey),
}

/// Named provider stores transcript paths must resolve inside, canonicalized at startup.
#[derive(Debug, Clone, Default)]
pub struct Stores {
    roots: BTreeMap<String, PathBuf>,
}

impl Stores {
    /// Authorizes each `(source name, directory)`, resolving the directory canonically.
    ///
    /// # Errors
    ///
    /// Returns the I/O failure of a directory that cannot be resolved.
    pub fn new(roots: &[(&str, PathBuf)]) -> std::io::Result<Self> {
        let roots = roots
            .iter()
            .map(|(name, root)| Ok(((*name).to_owned(), std::fs::canonicalize(root)?)))
            .collect::<std::io::Result<_>>()?;
        Ok(Self { roots })
    }

    /// Validates `session` into a typed reference.
    ///
    /// # Errors
    ///
    /// Returns `invalid_session` naming the offending field.
    pub fn source(&self, session: &SessionRef) -> Result<SourceRef, BindingError> {
        let invalid = |field| BindingError::InvalidSession { field };
        if session.source.is_empty() || session.source.len() > NAME_LIMIT {
            return Err(invalid("session.source"));
        }
        match session.kind.as_str() {
            "path" => {
                let root = self
                    .roots
                    .get(&session.source)
                    .ok_or_else(|| invalid("session.source"))?;
                transcript(root, &session.value).map(SourceRef::Transcript)
            }
            "id" if !session.value.is_empty() && session.value.len() <= NAME_LIMIT => {
                Ok(SourceRef::Session(SessionKey(session.value.clone())))
            }
            "id" => Err(invalid("session.value")),
            _ => Err(invalid("session.kind")),
        }
    }
}

/// Resolves `value`, absolute or relative to `root`, to a file inside `root`.
fn transcript(root: &Path, value: &str) -> Result<TranscriptPath, BindingError> {
    if value.is_empty() {
        return Err(BindingError::InvalidSession {
            field: "session.value",
        });
    }
    // Canonicalizing resolves every symlink and `..`, so the containment check below
    // cannot be escaped through a link planted inside the store.
    let canonical = std::fs::canonicalize(root.join(value))
        .map_err(|source| BindingError::SourceMissing { source })?;
    if !canonical.is_file() || !canonical.starts_with(root) {
        return Err(BindingError::OutsideStores);
    }
    Ok(TranscriptPath(canonical))
}
