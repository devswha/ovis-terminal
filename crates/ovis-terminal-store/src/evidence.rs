//! Durable recovery evidence recorded when a store cannot be opened as it is.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// What was found when the store refused to open its database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RecoveryCause {
    /// The file does not start with the SQLite header.
    CorruptHeader,
    /// SQLite reported the database or its integrity check as damaged.
    CorruptDatabase {
        /// SQLite's own description of the damage.
        detail: String,
    },
    /// A newer build wrote a schema version this build cannot read.
    UnsupportedSchema {
        /// The schema version stored in the database.
        found: i64,
        /// The newest schema version this build understands.
        supported: i64,
    },
}

/// Where the database and WAL material went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MaterialDisposition {
    /// The damaged files were moved out of the store root into the quarantine location.
    MovedToQuarantine,
    /// The files were copied to the quarantine location and left untouched in place.
    CopiedOriginalKept,
}

/// What the store knows about programs that were running when the evidence was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProcessSurvival {
    /// The store holds metadata only; it cannot tell whether any program survived.
    Unknown,
}

/// Whether recovery may start programs again from the stored metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RelaunchPolicy {
    /// No automatic relaunch: survival must first be settled by boot and process-incarnation evidence.
    RequiresIndependentEvidence,
}

/// The recovery flags carried by every piece of evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RecoveryFlags {
    /// Where the database and WAL material went.
    pub material: MaterialDisposition,
    /// What is known about program survival.
    pub process_survival: ProcessSurvival,
    /// Whether programs may be relaunched automatically.
    pub relaunch: RelaunchPolicy,
}

impl RecoveryFlags {
    pub(crate) const fn for_material(material: MaterialDisposition) -> Self {
        Self {
            material,
            process_survival: ProcessSurvival::Unknown,
            relaunch: RelaunchPolicy::RequiresIndependentEvidence,
        }
    }
}

/// One quarantine incident; reading it never clears or acknowledges it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RecoveryEvidence {
    /// The quarantine directory name, unique per incident.
    pub id: String,
    /// What was found.
    pub cause: RecoveryCause,
    /// The directory holding the preserved material and this evidence.
    pub location: PathBuf,
    /// File names preserved in `location`.
    pub preserved: Vec<String>,
    /// Wall-clock detection time; `None` when the clock reads before the Unix epoch.
    pub detected_at_unix_ms: Option<u64>,
    /// Recovery flags.
    pub flags: RecoveryFlags,
}
