//! Crash-atomic file replacement and quarantine moves.

use std::io::Write as _;
use std::path::{Path, PathBuf};

/// A failed step and its I/O error.
pub(super) type Failed = (&'static str, std::io::Error);

/// Replaces `path` with `bytes`: a crash leaves the old or the new content, never a mix.
pub(super) fn replace(path: &Path, bytes: &[u8]) -> Result<(), Failed> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| ("created", error))?;
    }
    let staged = path.with_extension("staged");
    let mut file = std::fs::File::create(&staged).map_err(|error| ("staged", error))?;
    file.write_all(bytes).map_err(|error| ("written", error))?;
    file.sync_all().map_err(|error| ("synced", error))?;
    drop(file);
    std::fs::rename(&staged, path).map_err(|error| ("published", error))?;
    sync_directory(path)
}

/// Moves `path` into `dir/quarantine/` under a name that never overwrites earlier
/// material, and returns where it went.
pub(super) fn quarantine(path: &Path, dir: &Path) -> Result<PathBuf, Failed> {
    let quarantine = dir.join("quarantine");
    std::fs::create_dir_all(&quarantine).map_err(|error| ("quarantined", error))?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let kept = quarantine.join(format!("{name}.{}", uuid::Uuid::now_v7()));
    std::fs::rename(path, &kept).map_err(|error| ("quarantined", error))?;
    sync_directory(&kept)?;
    Ok(kept)
}

/// Makes a rename durable; Windows commits it with the file itself.
#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), Failed> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::File::open(directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| ("made durable in its directory", error))
}

#[cfg(not(unix))]
const fn sync_directory(_: &Path) -> Result<(), Failed> {
    Ok(())
}
