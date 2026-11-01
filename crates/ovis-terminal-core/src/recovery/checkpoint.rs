//! Durable, offset-fenced screen checkpoints owned by the core.
//!
//! Each file is one header line and the engine's checkpoint bytes. The header binds the
//! body to a terminal incarnation, its output offset and a SHA-256 digest, so a torn,
//! flipped or foreign file is detected instead of being imported as a healthy screen;
//! such a file is moved to `quarantine/` and its location is returned.

use std::path::{Path, PathBuf};

use ovis_terminal_holder::protocol::TerminalTarget;
use ovis_terminal_protocol::schema::scalar::ByteOffset;
use ovis_terminal_vt::{Checkpoint, VtError};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::RecoveryError;
use super::durable::{Failed, quarantine, replace};

/// Format marker of the checkpoint header this build writes and reads.
pub const CHECKPOINT_FILE_FORMAT: &str = "ovis-terminal-recovery-checkpoint/1";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    format: String,
    target: TerminalTarget,
    offset: ByteOffset,
    body_sha256: String,
}

fn digest(body: &[u8]) -> String {
    let mut hex = String::with_capacity(64);
    for byte in Sha256::digest(body) {
        for nibble in [byte >> 4, byte & 0x0f] {
            hex.push(char::from_digit(u32::from(nibble), 16).unwrap_or('0'));
        }
    }
    hex
}

/// Checkpoints of every terminal incarnation, below one directory.
#[derive(Debug, Clone)]
pub struct CheckpointStore {
    dir: PathBuf,
}

impl CheckpointStore {
    /// Checkpoints kept under `dir`.
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }

    fn path(&self, target: &TerminalTarget) -> PathBuf {
        let id: String = target
            .terminal_id
            .as_str()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        self.dir
            .join(format!("{id}.{}.ckpt", target.terminal_incarnation.get()))
    }

    /// Stores `checkpoint` of `target` unless a newer one is already stored.
    ///
    /// # Errors
    ///
    /// [`RecoveryError::CheckpointStale`] for an older offset, or a persistence failure.
    pub fn save(
        &self,
        target: &TerminalTarget,
        checkpoint: &Checkpoint,
    ) -> Result<(), RecoveryError> {
        let offered = checkpoint.offset().get();
        match self.load(target) {
            Ok(Some(stored)) if stored.offset().get() > offered => {
                return Err(RecoveryError::CheckpointStale {
                    stored: stored.offset().get(),
                    offered,
                });
            }
            Ok(_)
            | Err(
                RecoveryError::CheckpointCorrupted { .. }
                | RecoveryError::CheckpointUnsupported { .. },
            ) => {}
            Err(error) => return Err(error),
        }
        let body = checkpoint.to_bytes()?;
        let header = Header {
            format: CHECKPOINT_FILE_FORMAT.to_owned(),
            target: target.clone(),
            offset: ByteOffset::new(offered),
            body_sha256: digest(&body),
        };
        let mut bytes = serde_json::to_vec(&header).map_err(VtError::CheckpointEncoding)?;
        bytes.push(b'\n');
        bytes.extend_from_slice(&body);
        replace(&self.path(target), &bytes).map_err(io)
    }

    /// Loads the checkpoint of `target`, if one is stored.
    ///
    /// # Errors
    ///
    /// A damaged file is moved to quarantine and reported as
    /// [`RecoveryError::CheckpointCorrupted`]; a file of another format as
    /// [`RecoveryError::CheckpointUnsupported`]; one naming another incarnation as
    /// [`RecoveryError::CheckpointTarget`].
    pub fn load(&self, target: &TerminalTarget) -> Result<Option<Checkpoint>, RecoveryError> {
        let path = self.path(target);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(io(("read", source))),
        };
        let split = bytes.iter().position(|byte| *byte == b'\n');
        let (head, body) = split.map_or((bytes.as_slice(), &[][..]), |at| {
            (
                bytes.get(..at).unwrap_or_default(),
                bytes.get(at.saturating_add(1)..).unwrap_or_default(),
            )
        });
        let Ok(header) = serde_json::from_slice::<Header>(head) else {
            return Err(self.corrupted(&path));
        };
        if header.format != CHECKPOINT_FILE_FORMAT {
            return Err(self.unsupported(&path));
        }
        if header.target != *target {
            return Err(RecoveryError::CheckpointTarget);
        }
        if header.body_sha256 != digest(body) {
            return Err(self.corrupted(&path));
        }
        match Checkpoint::from_bytes(body) {
            Ok(checkpoint) if checkpoint.offset().get() == header.offset.get() => {
                Ok(Some(checkpoint))
            }
            Err(VtError::CheckpointIncompatible { .. }) => Err(self.unsupported(&path)),
            Ok(_) | Err(_) => Err(self.corrupted(&path)),
        }
    }

    fn corrupted(&self, path: &Path) -> RecoveryError {
        match quarantine(path, &self.dir) {
            Ok(kept) => RecoveryError::CheckpointCorrupted { quarantine: kept },
            Err(failed) => io(failed),
        }
    }

    fn unsupported(&self, path: &Path) -> RecoveryError {
        match quarantine(path, &self.dir) {
            Ok(kept) => RecoveryError::CheckpointUnsupported { quarantine: kept },
            Err(failed) => io(failed),
        }
    }
}

fn io((operation, source): Failed) -> RecoveryError {
    RecoveryError::CheckpointIo { operation, source }
}
