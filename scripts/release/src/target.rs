//! Release targets, the host this tool runs on, and the native-host rule between them.

use serde::{Deserialize, Serialize};

use crate::error::ReleaseError;

/// The five published OS/architecture pairs, in manifest order.
pub(crate) const RELEASE_TARGETS: [&str; 5] = [
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
];

/// An operating system and CPU architecture, as Rust's `std::env::consts` names them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Platform {
    /// `linux`, `macos` or `windows`.
    pub(crate) os: String,
    /// `x86_64` or `aarch64`.
    pub(crate) arch: String,
}

impl Platform {
    /// The host this tool was built for and runs on.
    #[must_use]
    pub(crate) fn host() -> Self {
        Self {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
        }
    }

    /// The platform a target triple runs on.
    ///
    /// # Errors
    ///
    /// [`ReleaseError::UnknownTarget`] for an architecture or OS this release does not ship.
    pub(crate) fn of_triple(triple: &str) -> Result<Self, ReleaseError> {
        let unknown = || ReleaseError::UnknownTarget {
            triple: triple.to_owned(),
        };
        let arch = triple
            .split('-')
            .next()
            .filter(|arch| ["x86_64", "aarch64"].contains(arch));
        let os = if triple.contains("-linux-") {
            "linux"
        } else if triple.contains("-apple-darwin") {
            "macos"
        } else if triple.contains("-windows-") {
            "windows"
        } else {
            return Err(unknown());
        };
        Ok(Self {
            os: os.to_owned(),
            arch: arch.ok_or_else(unknown)?.to_owned(),
        })
    }

    /// `os/arch`.
    #[must_use]
    pub(crate) fn label(&self) -> String {
        format!("{}/{}", self.os, self.arch)
    }
}

/// Whether `triple` is one of the five published targets.
#[must_use]
pub(crate) fn is_release_target(triple: &str) -> bool {
    RELEASE_TARGETS.contains(&triple)
}

/// The binary's file name inside an artifact of `triple`.
#[must_use]
pub(crate) fn binary_name(triple: &str) -> &'static str {
    if triple.contains("-windows-") {
        "ovis-terminal.exe"
    } else {
        "ovis-terminal"
    }
}

#[cfg(test)]
mod tests {
    use super::{Platform, RELEASE_TARGETS, binary_name, is_release_target};

    #[test]
    fn every_release_target_maps_to_a_distinct_native_platform() {
        let platforms: Vec<String> = RELEASE_TARGETS
            .iter()
            .map(|triple| Platform::of_triple(triple).map(|platform| platform.label()))
            .collect::<Result<_, _>>()
            .unwrap_or_default();
        assert_eq!(
            platforms,
            [
                "linux/x86_64",
                "linux/aarch64",
                "macos/x86_64",
                "macos/aarch64",
                "windows/x86_64"
            ]
        );
    }

    #[test]
    fn an_unshipped_architecture_or_os_is_an_unknown_target() {
        for triple in ["riscv64gc-unknown-linux-gnu", "x86_64-unknown-freebsd"] {
            let kind = Platform::of_triple(triple).map_err(|error| error.kind());
            assert_eq!(kind, Err("unknown_target"), "{triple}");
        }
    }

    #[test]
    fn a_gnu_host_triple_is_stageable_but_never_a_release_target() {
        let platform = Platform::of_triple("x86_64-unknown-linux-gnu").map(|p| p.label());
        assert_eq!(platform.ok().as_deref(), Some("linux/x86_64"));
        assert!(!is_release_target("x86_64-unknown-linux-gnu"));
        assert_eq!(binary_name("x86_64-pc-windows-msvc"), "ovis-terminal.exe");
        assert_eq!(binary_name("aarch64-apple-darwin"), "ovis-terminal");
    }
}
