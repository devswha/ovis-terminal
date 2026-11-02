//! `seal` packs a staged artifact and records its archive digest; `verify` re-hashes every
//! staged file and archive against its entry.

use std::path::Path;

use serde_json::{Value, json};

use crate::docs::{ArchiveRecord, Entry};
use crate::error::ReleaseError;
use crate::files::{pack, read_json, sha256_file, unpack, write_json};
use crate::stage::records;

/// The directory holding `entry` (its staging directory).
fn staging(entry: &Path) -> &Path {
    entry.parent().unwrap_or_else(|| Path::new("."))
}

/// Requires the files under `dir` to be exactly the entry's files with their digests.
///
/// # Errors
///
/// [`ReleaseError::ChecksumMismatch`] naming the first differing, missing or extra file.
pub(crate) fn verify_files(dir: &Path, entry: &Entry) -> Result<(), ReleaseError> {
    let on_disk = records(&dir.join(&entry.name))?;
    for recorded in &entry.files {
        let actual = on_disk
            .iter()
            .find(|file| file.path == recorded.path)
            .map_or_else(|| "missing".to_owned(), |file| file.sha256.clone());
        if actual != recorded.sha256 {
            return Err(ReleaseError::ChecksumMismatch {
                file: format!("{}/{}", entry.name, recorded.path),
                recorded: recorded.sha256.clone(),
                actual,
            });
        }
    }
    if let Some(extra) = on_disk
        .iter()
        .find(|file| !entry.files.iter().any(|r| r.path == file.path))
    {
        return Err(ReleaseError::ChecksumMismatch {
            file: format!("{}/{}", entry.name, extra.path),
            recorded: "absent".to_owned(),
            actual: extra.sha256.clone(),
        });
    }
    Ok(())
}

/// Requires the sealed archive beside the entry, and its `.sha256` sidecar, to match the
/// entry's archive record.
///
/// # Errors
///
/// [`ReleaseError::Unsealed`] or [`ReleaseError::ChecksumMismatch`] naming the archive or
/// its sidecar.
pub(crate) fn verify_archive(dir: &Path, entry: &Entry) -> Result<ArchiveRecord, ReleaseError> {
    let archive = entry
        .archive
        .clone()
        .ok_or_else(|| ReleaseError::Unsealed {
            name: entry.name.clone(),
        })?;
    let (actual, _) = sha256_file(&dir.join(&archive.file))?;
    if actual != archive.sha256 {
        return Err(ReleaseError::ChecksumMismatch {
            file: archive.file,
            recorded: archive.sha256,
            actual,
        });
    }
    let sidecar = format!("{}.sha256", archive.file);
    let written = std::fs::read_to_string(dir.join(&sidecar)).unwrap_or_default();
    if written != sidecar_line(&archive.sha256, &archive.file) {
        let actual = written
            .split_whitespace()
            .next()
            .unwrap_or("missing")
            .to_owned();
        return Err(ReleaseError::ChecksumMismatch {
            file: sidecar,
            recorded: archive.sha256,
            actual,
        });
    }
    Ok(archive)
}

/// The `sha256sum`-format line of a `.sha256` sidecar.
pub(crate) fn sidecar_line(sha256: &str, file: &str) -> String {
    format!("{sha256}  {file}\n")
}

/// Unpacks the sealed archive into a scratch directory beside it and requires its files to
/// be exactly the entry's files, so the archive, not a staged directory, is what is checked.
///
/// # Errors
///
/// [`ReleaseError::ChecksumMismatch`] for a file the archive changed, added or lost, or
/// `tar` and file failures.
pub(crate) fn verify_unpacked(dir: &Path, entry: &Entry) -> Result<(), ReleaseError> {
    let archive = verify_archive(dir, entry)?;
    let scratch = dir.join(format!(".unpacked-{}", entry.name));
    if scratch.exists() {
        std::fs::remove_dir_all(&scratch).map_err(ReleaseError::io("clear", &scratch))?;
    }
    unpack(&dir.join(&archive.file), &scratch)?;
    let checked = verify_files(&scratch, entry);
    std::fs::remove_dir_all(&scratch).map_err(ReleaseError::io("remove", &scratch))?;
    checked
}

/// Re-verifies the staged files, packs `<name>.tar.gz` and records it in the entry.
///
/// # Errors
///
/// A changed staged file, or `tar` and file failures.
pub(crate) fn seal(entry_path: &Path) -> Result<Value, ReleaseError> {
    let mut entry: Entry = read_json(entry_path, "release entry")?;
    let dir = staging(entry_path);
    verify_files(dir, &entry)?;
    let file = pack(dir, &entry.name)?;
    let (sha256, bytes) = sha256_file(&dir.join(&file))?;
    entry.archive = Some(ArchiveRecord {
        file: file.clone(),
        sha256: sha256.clone(),
        bytes: bytes.clone(),
    });
    write_json(entry_path, &entry)?;
    std::fs::write(
        dir.join(format!("{file}.sha256")),
        sidecar_line(&sha256, &file),
    )
    .map_err(ReleaseError::io("write", dir))?;
    Ok(json!({ "sealed": file, "sha256": sha256, "bytes": bytes }))
}

/// Every `*.entry.json` of `dir`, sorted by file name.
///
/// # Errors
///
/// [`ReleaseError::Io`] or [`ReleaseError::Malformed`].
pub(crate) fn entries(dir: &Path) -> Result<Vec<Entry>, ReleaseError> {
    let listed: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map_err(ReleaseError::io("list", dir))?
        .map(|item| item.map(|item| item.path()))
        .collect::<Result<_, _>>()
        .map_err(ReleaseError::io("list", dir))?;
    let mut paths: Vec<std::path::PathBuf> = listed
        .into_iter()
        .filter(|path| path.to_string_lossy().ends_with(".entry.json"))
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|path| read_json(path, "release entry"))
        .collect()
}

/// Verifies every entry of `dir`: closed shape, the staged files when the staged directory
/// is present, and, when sealed, the archive, its sidecar and the archive's contents.
///
/// # Errors
///
/// The first malformed entry or digest mismatch.
pub(crate) fn verify(dir: &Path) -> Result<Value, ReleaseError> {
    let entries = entries(dir)?;
    let mut sealed = 0_usize;
    for entry in &entries {
        if dir.join(&entry.name).is_dir() {
            verify_files(dir, entry)?;
        }
        if entry.archive.is_some() {
            verify_unpacked(dir, entry)?;
            sealed = sealed.saturating_add(1);
        }
    }
    let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    Ok(json!({
        "verified": names,
        "entries": entries.len().to_string(),
        "sealed": sealed.to_string(),
    }))
}
