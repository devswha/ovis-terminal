//! Preserving damaged or newer database material and its durable evidence.

use std::fs::{self, File};
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::StoreError;
use crate::evidence::{MaterialDisposition, RecoveryCause, RecoveryEvidence, RecoveryFlags};

pub(crate) const DB_FILE: &str = "state.db";
const MATERIAL: [&str; 3] = ["state.db", "state.db-wal", "state.db-shm"];
const QUARANTINE_DIR: &str = "quarantine";
const EVIDENCE_FILE: &str = "evidence.json";
const MAX_SAME_MILLISECOND_INCIDENTS: u32 = 64;

/// Records the evidence first, then moves or copies the material next to it.
pub(crate) fn preserve(
    root: &Path,
    cause: RecoveryCause,
    material: MaterialDisposition,
) -> Result<RecoveryEvidence, StoreError> {
    let detected_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok());
    let (id, location) = create_incident_dir(root, detected_at_unix_ms)?;
    let preserved = MATERIAL
        .iter()
        .filter(|name| root.join(name).exists())
        .map(|name| (*name).to_owned())
        .collect();
    let evidence = RecoveryEvidence {
        id,
        cause,
        location,
        preserved,
        detected_at_unix_ms,
        flags: RecoveryFlags::for_material(material),
    };
    write_evidence(&evidence)?;
    for name in &evidence.preserved {
        let (from, to) = (root.join(name), evidence.location.join(name));
        match material {
            MaterialDisposition::MovedToQuarantine => {
                fs::rename(&from, &to).map_err(StoreError::io("move", &from))?;
            }
            MaterialDisposition::CopiedOriginalKept => copy_durably(&from, &to)?,
        }
    }
    sync_dir(&evidence.location)?;
    sync_dir(root)?;
    Ok(evidence)
}

/// Every recorded incident under `root`, oldest first; reading never removes one.
///
/// # Errors
///
/// Returns `persistence_failed` when the quarantine directory or an evidence file
/// cannot be read or decoded.
pub fn read_quarantine_evidence(root: &Path) -> Result<Vec<RecoveryEvidence>, StoreError> {
    let dir = root.join(QUARANTINE_DIR);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(StoreError::io("list", dir)(error)),
    };
    let mut incidents = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(StoreError::io("list", &dir))?
            .path()
            .join(EVIDENCE_FILE);
        let bytes = fs::read(&path).map_err(StoreError::io("read", &path))?;
        let evidence = serde_json::from_slice(&bytes)
            .map_err(|source| StoreError::Evidence { path, source })?;
        incidents.push(evidence);
    }
    incidents.sort_by(|a: &RecoveryEvidence, b| a.id.cmp(&b.id));
    Ok(incidents)
}

/// An earlier incident with the same cause, so repeated opens do not pile up copies.
pub(crate) fn find_incident(
    root: &Path,
    cause: &RecoveryCause,
) -> Result<Option<RecoveryEvidence>, StoreError> {
    let incidents = read_quarantine_evidence(root)?;
    Ok(incidents
        .into_iter()
        .rev()
        .find(|evidence| evidence.cause == *cause))
}

pub(crate) fn latest_incident(root: &Path) -> Result<Option<RecoveryEvidence>, StoreError> {
    Ok(read_quarantine_evidence(root)?.pop())
}

pub(crate) fn into_error(evidence: RecoveryEvidence) -> StoreError {
    let evidence = Box::new(evidence);
    match evidence.cause {
        RecoveryCause::UnsupportedSchema { .. } => StoreError::Unsupported { evidence },
        RecoveryCause::CorruptHeader | RecoveryCause::CorruptDatabase { .. } => {
            StoreError::Corrupted { evidence }
        }
    }
}

fn create_incident_dir(root: &Path, now_ms: Option<u64>) -> Result<(String, PathBuf), StoreError> {
    let parent = root.join(QUARANTINE_DIR);
    fs::create_dir_all(&parent).map_err(StoreError::io("create", &parent))?;
    let stamp = now_ms.map_or_else(|| "unknown-time".to_owned(), |ms| format!("{ms:013}"));
    for attempt in 0..MAX_SAME_MILLISECOND_INCIDENTS {
        let id = format!("{stamp}-{attempt:02}");
        let location = parent.join(&id);
        match fs::create_dir(&location) {
            Ok(()) => return Ok((id, location)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(StoreError::io("create", location)(error)),
        }
    }
    Err(StoreError::io("allocate an incident directory in", parent)(
        ErrorKind::AlreadyExists.into(),
    ))
}

fn write_evidence(evidence: &RecoveryEvidence) -> Result<(), StoreError> {
    let path = evidence.location.join(EVIDENCE_FILE);
    let bytes = serde_json::to_vec_pretty(evidence).map_err(|source| StoreError::Evidence {
        path: path.clone(),
        source,
    })?;
    let staging = evidence.location.join("evidence.json.tmp");
    let mut file = File::create(&staging).map_err(StoreError::io("create", &staging))?;
    file.write_all(&bytes)
        .map_err(StoreError::io("write", &staging))?;
    file.sync_all().map_err(StoreError::io("sync", &staging))?;
    fs::rename(&staging, &path).map_err(StoreError::io("publish", &path))?;
    sync_dir(&evidence.location)
}

fn copy_durably(from: &Path, to: &Path) -> Result<(), StoreError> {
    fs::copy(from, to).map_err(StoreError::io("copy", from))?;
    File::open(to)
        .and_then(|copy| copy.sync_all())
        .map_err(StoreError::io("sync", to))
}

#[cfg(unix)]
fn sync_dir(dir: &Path) -> Result<(), StoreError> {
    File::open(dir)
        .and_then(|handle| handle.sync_all())
        .map_err(StoreError::io("sync", dir))
}

/// Windows cannot open a directory handle for flushing; NTFS journals renames itself.
#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> Result<(), StoreError> {
    Ok(())
}
