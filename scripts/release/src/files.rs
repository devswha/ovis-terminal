//! Hashing, closed-document reads and writes, and helper programs.

use std::fs::File;
use std::io::{self, Read as _};
use std::path::Path;
use std::process::Command;

use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest as _, Sha256};

use crate::error::ReleaseError;

/// Lowercase hex SHA-256 of a file, and its length as a decimal string.
///
/// # Errors
///
/// [`ReleaseError::Io`] when the file cannot be read.
pub(crate) fn sha256_file(path: &Path) -> Result<(String, String), ReleaseError> {
    let mut file = File::open(path).map_err(ReleaseError::io("read", path))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(ReleaseError::io("read", path))?;
        let Some(chunk) = buffer.get(..read).filter(|chunk| !chunk.is_empty()) else {
            break;
        };
        hasher.update(chunk);
        total = total.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
    }
    Ok((hex(&hasher.finalize()), total.to_string()))
}

/// Lowercase hex SHA-256 of bytes.
#[must_use]
pub(crate) fn sha256_bytes(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        for nibble in [byte >> 4, byte & 0x0f] {
            text.extend(char::from_digit(u32::from(nibble), 16));
        }
    }
    text
}

/// Reads `path` as the closed document `T`.
///
/// # Errors
///
/// [`ReleaseError::Io`] or [`ReleaseError::Malformed`].
pub(crate) fn read_json<T: DeserializeOwned>(
    path: &Path,
    document: &'static str,
) -> Result<T, ReleaseError> {
    let bytes = std::fs::read(path).map_err(ReleaseError::io("read", path))?;
    serde_json::from_slice(&bytes).map_err(|source| ReleaseError::Malformed {
        path: path.to_path_buf(),
        document,
        source,
    })
}

/// Writes `value` as pretty JSON with a trailing newline.
///
/// # Errors
///
/// [`ReleaseError::Io`] when the file cannot be written.
pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), ReleaseError> {
    let mut text =
        serde_json::to_string_pretty(value).map_err(|source| ReleaseError::Malformed {
            path: path.to_path_buf(),
            document: "JSON document",
            source,
        })?;
    text.push('\n');
    std::fs::write(path, text).map_err(ReleaseError::io("write", path))
}

/// Copies `from` to `to`.
///
/// # Errors
///
/// [`ReleaseError::Io`] naming the source.
pub(crate) fn copy(from: &Path, to: &Path) -> Result<(), ReleaseError> {
    std::fs::copy(from, to)
        .map(drop)
        .map_err(ReleaseError::io("copy", from))
}

/// Runs a helper program to completion and returns its stdout; a non-zero exit is an error.
///
/// # Errors
///
/// [`ReleaseError::Tool`] when it cannot start or exits non-zero.
pub(crate) fn helper(command: &mut Command) -> Result<Vec<u8>, ReleaseError> {
    let program = format!("{command:?}");
    let failed = |detail: String| ReleaseError::Tool {
        program: program.clone(),
        detail,
    };
    let output = command
        .output()
        .map_err(|error: io::Error| failed(error.to_string()))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(failed(format!(
            "{}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

/// `tar`; on Windows the system bsdtar, which reads drive-letter paths that the MSYS GNU tar
/// on a Git Bash `PATH` would take for remote hosts.
fn tar() -> Command {
    let system = std::env::var_os("SystemRoot")
        .filter(|_| cfg!(windows))
        .map(|root| {
            std::path::PathBuf::from(root)
                .join("System32")
                .join("tar.exe")
        });
    Command::new(system.unwrap_or_else(|| "tar".into()))
}

/// Packs `<parent>/<name>` into `<parent>/<name>.tar.gz` and returns the archive's file name.
///
/// # Errors
///
/// [`ReleaseError::Tool`] when `tar` fails.
pub(crate) fn pack(parent: &Path, name: &str) -> Result<String, ReleaseError> {
    let archive = format!("{name}.tar.gz");
    helper(tar().current_dir(parent).args(["-czf", &archive, name]))?;
    Ok(archive)
}

/// Unpacks a gzip tar archive into `into`.
///
/// # Errors
///
/// [`ReleaseError::Tool`] when `tar` fails, [`ReleaseError::Io`] when `into` cannot be made.
pub(crate) fn unpack(archive: &Path, into: &Path) -> Result<(), ReleaseError> {
    std::fs::create_dir_all(into).map_err(ReleaseError::io("create", into))?;
    helper(tar().arg("-xzf").arg(archive).arg("-C").arg(into)).map(drop)
}
