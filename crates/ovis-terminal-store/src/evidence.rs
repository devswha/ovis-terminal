//! Durable recovery evidence recorded when a store cannot be opened as it is.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The identity of one quarantine incident: its directory name, which sorts by detection time.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IncidentId(String);

impl IncidentId {
    pub(crate) const fn new(id: String) -> Self {
        Self(id)
    }

    /// The incident id text, also the quarantine directory name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IncidentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

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
    /// The incident identity, unique per incident.
    pub id: IncidentId,
    /// What was found.
    pub cause: RecoveryCause,
    /// The directory holding the preserved material and this evidence.
    pub location: PathBuf,
    /// File names preserved in `location`.
    pub preserved: Vec<String>,
    /// Wall-clock detection time; `None` when the clock reads before the Unix epoch.
    pub detected_at_unix_ms: Option<UnixMillis>,
    /// Recovery flags.
    pub flags: RecoveryFlags,
}

/// Milliseconds since the Unix epoch, written to durable JSON as a canonical decimal string
/// so readers limited to 53-bit integers never lose precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnixMillis(u64);

impl UnixMillis {
    pub(crate) const fn new(millis: u64) -> Self {
        Self(millis)
    }

    /// The raw millisecond count.
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl Serialize for UnixMillis {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for UnixMillis {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        let text = String::deserialize(deserializer)?;
        let millis: u64 = text
            .parse()
            .map_err(|_| D::Error::custom(format!("not a decimal u64: {text:?}")))?;
        if millis.to_string() != text {
            return Err(D::Error::custom(format!(
                "non-canonical decimal u64: {text:?}"
            )));
        }
        Ok(Self(millis))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        IncidentId, MaterialDisposition, RecoveryCause, RecoveryEvidence, RecoveryFlags, UnixMillis,
    };

    fn evidence_at(detected_at_unix_ms: Option<UnixMillis>) -> RecoveryEvidence {
        RecoveryEvidence {
            id: IncidentId::new("0000000000001-00".to_owned()),
            cause: RecoveryCause::CorruptHeader,
            location: PathBuf::from("quarantine/0000000000001-00"),
            preserved: vec!["state.db".to_owned()],
            detected_at_unix_ms,
            flags: RecoveryFlags::for_material(MaterialDisposition::MovedToQuarantine),
        }
    }

    #[test]
    fn timestamps_beyond_javascript_precision_round_trip_as_strings() {
        let original = evidence_at(Some(UnixMillis::new(u64::MAX)));

        let json = serde_json::to_value(&original).unwrap();

        assert_eq!(
            json.get("detected_at_unix_ms"),
            Some(&serde_json::json!("18446744073709551615"))
        );
        assert_eq!(
            serde_json::from_value::<RecoveryEvidence>(json).unwrap(),
            original
        );
    }

    #[test]
    fn non_canonical_or_numeric_timestamps_are_rejected() {
        let mut json = serde_json::to_value(evidence_at(Some(UnixMillis::new(7)))).unwrap();
        for bad in [
            serde_json::json!("007"),
            serde_json::json!("+7"),
            serde_json::json!(7),
        ] {
            *json.get_mut("detected_at_unix_ms").unwrap() = bad.clone();

            let parsed = serde_json::from_value::<RecoveryEvidence>(json.clone());

            assert!(parsed.is_err(), "{bad} was accepted");
        }
    }
}
