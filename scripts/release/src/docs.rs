//! The closed documents a release is made of: pins, receipts, entries, QA and manifest.
//! Every byte count is a decimal string; unknown fields are refused.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::target::Platform;

/// `scripts/release/pins.json`: the toolchain every release target must be built with.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pins {
    /// `ovis-release-pins/1`.
    pub(crate) format: String,
    /// The Rust toolchain; equals `rust-toolchain.toml`.
    pub(crate) rust: String,
    /// The SQLite amalgamation rusqlite bundles.
    pub(crate) sqlite: String,
    /// The libsqlite3-sys version that carries it.
    pub(crate) libsqlite3_sys: String,
    /// What is and is not signed.
    pub(crate) signing: Signing,
    /// Per release target: its native runner and C toolchain.
    pub(crate) targets: BTreeMap<String, TargetPin>,
}

/// Signing and notarization status, stated rather than implied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Signing {
    /// Whether any artifact is code-signed or notarized.
    pub(crate) performed: bool,
    /// The statement published with the release.
    pub(crate) statement: String,
}

/// One target's pinned runner and C toolchain observations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TargetPin {
    /// The GitHub-hosted runner label that is the target's native host.
    pub(crate) runner: String,
    /// Observation name to the exact value the build receipt must carry.
    pub(crate) c_toolchain: BTreeMap<String, String>,
}

/// What a build job observed about the host and toolchain that built one artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Receipt {
    /// `ovis-release-receipt/1`.
    pub(crate) format: String,
    /// The target triple built.
    pub(crate) target: String,
    /// The build host.
    pub(crate) host: Platform,
    /// The runner label, or `local`.
    pub(crate) runner: String,
    /// `rustc` release, such as `1.99.1`.
    pub(crate) rustc: String,
    /// The bundled SQLite version read from the libsqlite3-sys source.
    pub(crate) sqlite: String,
    /// The libsqlite3-sys version in Cargo.lock.
    pub(crate) libsqlite3_sys: String,
    /// C compiler, linker and SDK observations, by name.
    pub(crate) c_toolchain: BTreeMap<String, String>,
}

/// One file inside an artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileRecord {
    /// Path inside the artifact directory.
    pub(crate) path: String,
    /// Lowercase hex SHA-256.
    pub(crate) sha256: String,
    /// Length in bytes, decimal.
    pub(crate) bytes: String,
}

/// What the stage checked before it wrote the entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Checks {
    /// The binary prints `schema/ovis-terminal.openrpc.json` byte for byte.
    pub(crate) schema_matches_source: bool,
    /// Part name to the version it reports; all equal the entry version.
    pub(crate) versions: BTreeMap<String, String>,
    /// The binary's `schema compat --released` invocation record.
    pub(crate) compat: Value,
    /// Registry crates the notices inventory lists, decimal.
    pub(crate) notice_crates: String,
    /// The Apache-2.0 attribution and license text are present.
    pub(crate) apache_attribution: bool,
}

/// The sealed archive of an artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchiveRecord {
    /// File name beside the entry.
    pub(crate) file: String,
    /// Lowercase hex SHA-256.
    pub(crate) sha256: String,
    /// Length in bytes, decimal.
    pub(crate) bytes: String,
}

/// `<name>.entry.json`: one staged target artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    /// `ovis-release-entry/1`.
    pub(crate) format: String,
    /// `ovis-terminal-<version>-<target>`, the artifact directory and archive stem.
    pub(crate) name: String,
    /// The release version.
    pub(crate) version: String,
    /// The protocol generation.
    pub(crate) generation: String,
    /// The target triple.
    pub(crate) target: String,
    /// Whether the target is one of the five published ones.
    pub(crate) release_target: bool,
    /// Every file of the artifact directory.
    pub(crate) files: Vec<FileRecord>,
    /// The stage's checks.
    pub(crate) checks: Checks,
    /// The build receipt.
    pub(crate) receipt: Receipt,
    /// Set by `seal`.
    pub(crate) archive: Option<ArchiveRecord>,
}

/// One check of a native QA run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QaCheck {
    /// The check name.
    pub(crate) check: String,
    /// Whether it held.
    pub(crate) passed: bool,
    /// The invocation record or the mismatch.
    pub(crate) detail: Value,
}

/// `<name>.qa.json`: the artifact unpacked and run on its native host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QaReceipt {
    /// `ovis-release-qa/1`.
    pub(crate) format: String,
    /// The entry name.
    pub(crate) name: String,
    /// The target triple.
    pub(crate) target: String,
    /// The host the checks ran on.
    pub(crate) host: Platform,
    /// The runner label, or `local`.
    pub(crate) runner: String,
    /// The archive that was unpacked.
    pub(crate) archive_sha256: String,
    /// Every check, in order.
    pub(crate) checks: Vec<QaCheck>,
    /// Every check held.
    pub(crate) passed: bool,
}

/// One target of a promoted release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManifestTarget {
    /// The target triple.
    pub(crate) target: String,
    /// Its native OS and architecture, the only pair it is supported on.
    pub(crate) platform: Platform,
    /// The archive file.
    pub(crate) archive: String,
    /// Its SHA-256.
    pub(crate) sha256: String,
    /// Its length, decimal.
    pub(crate) bytes: String,
    /// The runner that built and checked it.
    pub(crate) runner: String,
    /// The QA receipt digest.
    pub(crate) qa_sha256: String,
}

/// `release-manifest.json`: what a promotion publishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    /// `ovis-release-manifest/1`.
    pub(crate) format: String,
    /// The version.
    pub(crate) version: String,
    /// The protocol generation.
    pub(crate) generation: String,
    /// Signing and notarization status.
    pub(crate) signing: Signing,
    /// The supported targets, in [`crate::target::RELEASE_TARGETS`] order.
    pub(crate) targets: Vec<ManifestTarget>,
}
