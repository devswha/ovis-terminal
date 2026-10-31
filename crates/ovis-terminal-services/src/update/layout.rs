//! Install-root paths, update identities and durable whole-file replacement.
//!
//! ```text
//! <root>/active.json            selected build and its rollback target
//! <root>/activation.json        journal of an activation or rollback in progress
//! <root>/updates/<id>.json      one record per staged update
//! <root>/staging/<id>/archive.tar
//! <root>/builds/<id>/           extracted, verified build
//! ```

use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use ovis_terminal_protocol::domain::update::BuildTarget;
use ovis_terminal_protocol::schema::ids::UpdateId;
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use super::UpdateError;

const ID_PREFIX: &str = "update-";

/// Paths below one install root.
#[derive(Debug, Clone)]
pub(super) struct Layout {
    root: PathBuf,
}

impl Layout {
    pub(super) fn create(root: PathBuf) -> Result<Self, UpdateError> {
        for dir in ["updates", "staging", "builds"] {
            fs::create_dir_all(root.join(dir)).map_err(io("create the install root"))?;
        }
        Ok(Self { root })
    }

    pub(super) fn selection(&self) -> PathBuf {
        self.root.join("active.json")
    }

    pub(super) fn journal(&self) -> PathBuf {
        self.root.join("activation.json")
    }

    pub(super) fn records(&self) -> PathBuf {
        self.root.join("updates")
    }

    pub(super) fn record(&self, id: &UpdateId) -> PathBuf {
        self.records().join(format!("{}.json", id.as_str()))
    }

    pub(super) fn staging(&self, id: &UpdateId) -> PathBuf {
        self.root.join("staging").join(id.as_str())
    }

    pub(super) fn archive(&self, id: &UpdateId) -> PathBuf {
        self.staging(id).join("archive.tar")
    }

    pub(super) fn build(&self, id: &UpdateId) -> PathBuf {
        self.root.join("builds").join(id.as_str())
    }
}

/// A fresh, time-ordered update identity that is also a safe file name.
pub(super) fn new_update_id() -> UpdateId {
    UpdateId::new(format!("{ID_PREFIX}{}", Uuid::now_v7()))
}

/// Whether a caller-supplied identity has the only shape this service mints.
pub(super) fn is_update_id(id: &UpdateId) -> bool {
    id.as_str()
        .strip_prefix(ID_PREFIX)
        .and_then(|uuid| Uuid::try_parse(uuid).ok())
        .is_some_and(|uuid| format!("{ID_PREFIX}{uuid}") == id.as_str())
}

/// The executable's file name inside a build for `target`.
pub(super) fn binary_name(target: &BuildTarget) -> &'static str {
    if target.as_str().starts_with("windows-") {
        "ovis-terminal.exe"
    } else {
        "ovis-terminal"
    }
}

pub(super) const fn io(operation: &'static str) -> impl FnOnce(std::io::Error) -> UpdateError {
    move |source| UpdateError::Io { operation, source }
}

/// Replaces `path` with `value` so a crash leaves the old or the new file, never a mix.
pub(super) fn store<T: Serialize>(path: &Path, value: &T) -> Result<(), UpdateError> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let partial = path.with_extension(format!("partial-{}", Uuid::now_v7()));
    let mut file = File::create_new(&partial).map_err(io("create a record"))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(io("write a record"))?;
    fs::rename(&partial, path).map_err(io("replace a record"))?;
    sync_parent(path)
}

/// Reads `path`, or `None` when it does not exist.
pub(super) fn load<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, UpdateError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io("read a record")(error)),
    }
}

/// Removes `path`; an already absent file is the wanted outcome.
pub(super) fn remove(path: &Path) -> Result<(), UpdateError> {
    match fs::remove_file(path) {
        Ok(()) => sync_parent(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io("remove a record")(error)),
    }
}

/// Removes the directory tree at `path` if it exists.
pub(super) fn remove_tree(path: &Path) -> Result<(), UpdateError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io("remove partial files")(error)),
    }
}

/// Makes a rename inside `path`'s directory durable.
#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<(), UpdateError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(io("sync the install root"))
}

/// Windows commits a rename with the file's own metadata; directories cannot be opened.
#[cfg(not(unix))]
fn sync_parent(path: &Path) -> Result<(), UpdateError> {
    let _: &Path = path;
    Ok(())
}
