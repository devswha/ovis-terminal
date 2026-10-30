//! Discovery of the Rust sources that the hygiene rules apply to.

use std::path::{Path, PathBuf};
use std::{fs, io};

/// Directory names never descended into: build output, VCS metadata, package
/// caches, local orchestration state and the gitignored upstream reference clone.
const SKIPPED_DIRS: &[&str] = &["target", ".git", "node_modules", ".omo", "herdr-core"];

/// Fixture directory whose files violate the rules on purpose.
const FIXTURE_DIR: &[&str] = &["scripts", "hygiene", "fixtures"];

/// Failure to list or read workspace files.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WalkError {
    /// A directory or file could not be read.
    #[error("cannot read {path}: {source}", path = path.display())]
    Io {
        /// Path that failed.
        path: PathBuf,
        /// Underlying I/O error.
        source: io::Error,
    },
}

/// Lists every `.rs` file under `root`, sorted, excluding skipped directories
/// and the checker's own fixtures.
///
/// # Errors
///
/// Returns [`WalkError::Io`] when a directory cannot be listed.
pub fn rust_sources(root: &Path) -> Result<Vec<PathBuf>, WalkError> {
    let fixtures: PathBuf = FIXTURE_DIR.iter().collect();
    let fixture_dir = root.join(fixtures);
    let mut pending = vec![root.to_path_buf()];
    let mut found = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in read_dir(&dir)? {
            let path = entry.path();
            let is_dir = entry
                .file_type()
                .map_err(|source| WalkError::Io {
                    path: path.clone(),
                    source,
                })?
                .is_dir();
            if is_dir {
                if !is_skipped(&path) && path != fixture_dir {
                    pending.push(path);
                }
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Reads one source file as UTF-8 text.
///
/// # Errors
///
/// Returns [`WalkError::Io`] when the file cannot be read or is not UTF-8.
pub fn read_source(path: &Path) -> Result<String, WalkError> {
    fs::read_to_string(path).map_err(|source| WalkError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn read_dir(dir: &Path) -> Result<Vec<fs::DirEntry>, WalkError> {
    let io_error = |source| WalkError::Io {
        path: dir.to_path_buf(),
        source,
    };
    fs::read_dir(dir)
        .map_err(io_error)?
        .collect::<Result<_, _>>()
        .map_err(io_error)
}

fn is_skipped(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| SKIPPED_DIRS.iter().any(|skip| name == *skip))
}
