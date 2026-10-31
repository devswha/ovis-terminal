//! The durable restart intent: written before any step, removed only after recovery.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use ovis_terminal_protocol::domain::restart::{InterruptedWait, RestartPhase};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use serde::{Deserialize, Serialize};

use super::{Preserved, RestartError};
use crate::registry::{HostBootId, RuntimeId, ServerBootId};

/// Format marker of the intent file; another marker is refused, never guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum IntentFormat {
    /// The first format.
    #[serde(rename = "ovis-terminal-restart-intent/1")]
    V1,
}

/// Everything a successor needs to finish a restart without a cached endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RestartIntent {
    /// Format marker.
    pub format: IntentFormat,
    /// The `server.restart` operation.
    pub op_id: OpId,
    /// Host boot the intent was written under.
    pub host_boot: HostBootId,
    /// Runtime lifetime the successor keeps.
    pub runtime: RuntimeId,
    /// Boot identity being replaced.
    pub from: ServerBootId,
    /// Boot identity the successor takes.
    pub to: ServerBootId,
    /// Last barrier crossed.
    pub phase: RestartPhase,
    /// Incarnations owned when the restart began.
    pub preserved: Vec<Preserved>,
    /// Operations in flight when the restart began.
    pub in_flight: Vec<OpId>,
    /// Waits interrupted at the boundary.
    pub waits: Vec<InterruptedWait>,
    /// Latest committed sequence when the intent was written.
    pub seq: EventSeq,
}

impl RestartIntent {
    /// An intent of `op_id` at [`RestartPhase::IntentPersisted`] with nothing recorded yet.
    #[must_use]
    pub const fn new(
        op_id: OpId,
        (host_boot, runtime): (HostBootId, RuntimeId),
        (from, to): (ServerBootId, ServerBootId),
    ) -> Self {
        Self {
            format: IntentFormat::V1,
            op_id,
            host_boot,
            runtime,
            from,
            to,
            phase: RestartPhase::IntentPersisted,
            preserved: Vec::new(),
            in_flight: Vec::new(),
            waits: Vec::new(),
            seq: EventSeq::new(0),
        }
    }
}

/// The intent file in a state directory.
#[derive(Debug, Clone)]
pub struct IntentFile {
    path: PathBuf,
}

impl IntentFile {
    /// The intent file of the state directory `dir`.
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join("restart-intent.json"),
        }
    }

    /// Replaces the intent atomically: a crash leaves the old or the new one, never a mix.
    ///
    /// # Errors
    ///
    /// [`RestartError::IntentIo`] when the file or its directory cannot be synced.
    pub fn store(&self, intent: &RestartIntent) -> Result<(), RestartError> {
        let bytes = serde_json::to_vec(intent).map_err(RestartError::IntentMalformed)?;
        let staged = self.path.with_extension("json.staged");
        let io = |operation| move |source| RestartError::IntentIo { operation, source };
        let mut file = std::fs::File::create(&staged).map_err(io("staged"))?;
        file.write_all(&bytes).map_err(io("written"))?;
        file.sync_all().map_err(io("synced"))?;
        drop(file);
        std::fs::rename(&staged, &self.path).map_err(io("published"))?;
        #[cfg(unix)]
        sync_directory(&self.path)?;
        Ok(())
    }

    /// Reads the intent, if one is pending.
    ///
    /// # Errors
    ///
    /// [`RestartError::IntentMalformed`] for a file that is not this exact shape; the
    /// file is kept for inspection.
    pub fn load(&self) -> Result<Option<RestartIntent>, RestartError> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(RestartError::IntentMalformed),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(RestartError::IntentIo {
                operation: "read",
                source,
            }),
        }
    }

    /// Removes the intent once the successor recovered.
    ///
    /// # Errors
    ///
    /// [`RestartError::IntentIo`] when it cannot be removed durably.
    pub fn clear(&self) -> Result<(), RestartError> {
        std::fs::remove_file(&self.path).map_err(|source| RestartError::IntentIo {
            operation: "removed",
            source,
        })?;
        #[cfg(unix)]
        sync_directory(&self.path)?;
        Ok(())
    }
}

/// Makes a rename or removal durable; Windows commits it with the file itself.
#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), RestartError> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::File::open(directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| RestartError::IntentIo {
            operation: "made durable in its directory",
            source,
        })
}

#[cfg(test)]
mod tests {
    use ovis_terminal_protocol::schema::ids::TerminalId;
    use ovis_terminal_protocol::schema::scalar::ByteOffset;

    use super::*;

    fn intent() -> RestartIntent {
        let mut intent = RestartIntent::new(
            OpId::new("0191f1a2-0000-7000-8000-000000000048".to_owned()),
            (
                HostBootId::new(std::num::NonZeroU64::MIN),
                RuntimeId::new(std::num::NonZeroU64::MAX),
            ),
            (
                ServerBootId::new(std::num::NonZeroU64::MIN),
                ServerBootId::new(std::num::NonZeroU64::MAX),
            ),
        );
        intent.seq = EventSeq::new(u64::MAX);
        intent.waits.push(InterruptedWait::new(
            OpId::new("wait".to_owned()),
            TerminalId::new("terminal:4".to_owned()),
            ByteOffset::new(u64::MAX),
        ));
        intent
    }

    fn root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("ovis-t48-intent-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&root).unwrap_or_default();
        root
    }

    #[test]
    fn an_intent_survives_a_reopen_exactly_and_clearing_leaves_none() -> Result<(), RestartError> {
        let root = root();
        let file = IntentFile::new(&root);
        assert_eq!(file.load()?, None);
        file.store(&intent())?;
        assert_eq!(IntentFile::new(&root).load()?, Some(intent()));
        file.clear()?;
        assert_eq!(file.load()?, None);
        std::fs::remove_dir_all(root).map_err(|source| RestartError::IntentIo {
            operation: "cleaned",
            source,
        })
    }

    #[test]
    fn counters_are_decimal_strings_and_a_numeric_or_foreign_intent_is_refused() {
        let value = serde_json::to_value(intent()).unwrap_or_default();
        assert_eq!(
            value.get("seq"),
            Some(&serde_json::json!("18446744073709551615"))
        );
        assert_eq!(
            value.get("format"),
            Some(&serde_json::json!("ovis-terminal-restart-intent/1"))
        );
        let with = |key: &str, field: serde_json::Value| {
            let mut changed = value.clone();
            if let Some(object) = changed.as_object_mut() {
                object.insert(key.to_owned(), field);
            }
            changed
        };
        let numeric = with("seq", serde_json::json!(7));
        let foreign = with(
            "format",
            serde_json::json!("ovis-terminal-restart-intent/2"),
        );
        let extra = with("guess", serde_json::json!(true));
        for refused in [numeric, foreign, extra] {
            let root = root();
            std::fs::write(root.join("restart-intent.json"), refused.to_string())
                .unwrap_or_default();
            let read = IntentFile::new(&root).load().map_err(|error| error.kind());
            assert_eq!(read, Err("intent_malformed"));
            assert!(
                root.join("restart-intent.json").exists(),
                "a refused intent is kept"
            );
            std::fs::remove_dir_all(root).unwrap_or_default();
        }
    }
}
