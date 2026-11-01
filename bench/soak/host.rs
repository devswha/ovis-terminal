//! Host, build and binary-under-test facts every report states.

use std::path::Path;
use std::process::Command;

use serde::Serialize;
use sha2::{Digest as _, Sha256};

use super::error::SoakError;

#[derive(Debug, Clone, Serialize)]
pub(super) struct Host {
    pub(super) os: &'static str,
    pub(super) arch: &'static str,
    pub(super) kernel: String,
    pub(super) available_parallelism: usize,
    pub(super) build_profile: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Binary {
    pub(super) path: String,
    pub(super) sha256: String,
    pub(super) bytes: u64,
    pub(super) version: String,
}

pub(super) fn host() -> Host {
    let kernel = Command::new("uname")
        .arg("-srm")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map_or_else(
            || "unavailable: uname -srm".to_owned(),
            |output| String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        );
    Host {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        kernel,
        available_parallelism: std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get),
        build_profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
    }
}

/// Hash, size and `--version` of the binary under test.
pub(super) fn binary(path: &Path) -> Result<Binary, SoakError> {
    let bytes = std::fs::read(path).map_err(|source| SoakError::Binary { source })?;
    let version = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|source| SoakError::Binary { source })?;
    Ok(Binary {
        path: path.display().to_string(),
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        version: String::from_utf8_lossy(&version.stdout).trim().to_owned(),
    })
}
