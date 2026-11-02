//! The release-stage boundary error: every refusal has a stable kind, a hint and an exit.

use std::io;
use std::path::PathBuf;

use serde_json::{Value, json};
use thiserror::Error;

/// Why a release stage refused or could not run.
#[derive(Debug, Error)]
pub(crate) enum ReleaseError {
    /// The arguments do not parse.
    #[error("the command line does not parse: {detail}")]
    Usage {
        /// Clap's first diagnostic line.
        detail: String,
    },
    /// A file could not be read, written or listed.
    #[error("cannot {operation} {path}")]
    Io {
        /// What was attempted.
        operation: &'static str,
        /// The path involved.
        path: PathBuf,
        /// The failure.
        #[source]
        source: io::Error,
    },
    /// A manifest, entry, receipt or pins file is not the closed document it must be.
    #[error("{path} is not a well-formed {document}")]
    Malformed {
        /// The file.
        path: PathBuf,
        /// Which document format it should be.
        document: &'static str,
        /// The parse failure.
        #[source]
        source: serde_json::Error,
    },
    /// A file's bytes do not hash to what its manifest records.
    #[error("{file} does not match its recorded sha256")]
    ChecksumMismatch {
        /// The file, relative to the release directory.
        file: String,
        /// The recorded digest.
        recorded: String,
        /// The digest of the bytes on disk.
        actual: String,
    },
    /// Two parts of one release disagree on the version.
    #[error("{part} says version {found}, the release is {expected}")]
    VersionMismatch {
        /// Which part disagrees.
        part: &'static str,
        /// The release version.
        expected: String,
        /// What the part says.
        found: String,
    },
    /// The binary's OpenRPC document is not the source tree's.
    #[error("the binary's OpenRPC document differs from schema/ovis-terminal.openrpc.json")]
    SchemaMismatch,
    /// The binary reports that it breaks the released baseline.
    #[error("the built binary breaks the released baseline")]
    CompatibilityBreak {
        /// The binary's own invocation record, including its error.
        binary: Value,
    },
    /// The built binary failed a check it must pass.
    #[error("`{command}` exited {exit}")]
    BinaryFailed {
        /// The invocation.
        command: String,
        /// Its exit status (`-1` when it was killed by a signal).
        exit: i32,
        /// The invocation record.
        binary: Value,
    },
    /// `THIRD_PARTY_NOTICES.md` lacks the Apache-2.0 attribution or the license text.
    #[error("the notices lack {missing}")]
    NoticesMissing {
        /// The missing part.
        missing: &'static str,
    },
    /// The crate inventory does not list every crate of Cargo.lock.
    #[error("the notices inventory misses {} crate(s)", .crates.len())]
    NoticesIncomplete {
        /// `name version` of every unlisted crate.
        crates: Vec<String>,
    },
    /// The inventory differs from what `cargo metadata` reports now.
    #[error("the notices inventory is not what `release-stage notices` generates")]
    NoticesStale,
    /// A release target has no artifact entry.
    #[error("no artifact for {}", .targets.join(", "))]
    TargetMissing {
        /// The missing targets.
        targets: Vec<String>,
    },
    /// A target has no native QA receipt.
    #[error("no native QA receipt for {target}")]
    NativeQaMissing {
        /// The target.
        target: String,
    },
    /// A target's native QA failed or ran on another OS or architecture.
    #[error("native QA of {target} did not pass natively")]
    NativeQaFailed {
        /// The target.
        target: String,
        /// The failed check names.
        failed: Vec<String>,
    },
    /// QA was asked to run on a host that is not the target.
    #[error("this host is {host}; {target} needs its own native host")]
    NotNative {
        /// The target.
        target: String,
        /// `os/arch` of this host.
        host: String,
    },
    /// A build receipt names a host that is not the target: a cross build.
    #[error("{target} was built on {host}; release builds run on their native host")]
    CrossBuildRefused {
        /// The target.
        target: String,
        /// `os/arch` of the build host.
        host: String,
    },
    /// A build receipt does not match the pinned toolchain.
    #[error("the receipt of {target} has {field} = {found:?}, pinned {pinned:?}")]
    ReceiptUnpinned {
        /// The target.
        target: String,
        /// The receipt field.
        field: String,
        /// The pinned value.
        pinned: String,
        /// The receipt value.
        found: String,
    },
    /// The entry has no sealed archive yet.
    #[error("{name} has no sealed archive")]
    Unsealed {
        /// The entry name.
        name: String,
    },
    /// The triple is not one this tool knows.
    #[error("{triple} is not a target this release knows")]
    UnknownTarget {
        /// The triple.
        triple: String,
    },
    /// The version was already published with other artifacts.
    #[error("version {version} is already published with a different manifest")]
    ReleaseConflict {
        /// The version.
        version: String,
    },
    /// A helper program (tar, cargo, rustc) could not run or failed.
    #[error("`{program}` failed")]
    Tool {
        /// The program and its arguments.
        program: String,
        /// Its exit status, or the spawn failure.
        detail: String,
    },
    /// The fixture name is not one this tool seeds.
    #[error("{name} is not a release fixture")]
    UnknownFixture {
        /// The name.
        name: String,
    },
}

impl ReleaseError {
    /// The stable machine-readable kind.
    #[must_use]
    pub(crate) const fn kind(&self) -> &'static str {
        match self {
            Self::Usage { .. } => "usage",
            Self::Io { .. } => "io",
            Self::Malformed { .. } => "manifest_malformed",
            Self::ChecksumMismatch { .. } => "checksum_mismatch",
            Self::VersionMismatch { .. } => "version_mismatch",
            Self::SchemaMismatch => "schema_mismatch",
            Self::CompatibilityBreak { .. } => "compatibility_break",
            Self::BinaryFailed { .. } => "binary_failed",
            Self::NoticesMissing { .. } => "notices_missing",
            Self::NoticesIncomplete { .. } => "notices_incomplete",
            Self::NoticesStale => "notices_stale",
            Self::TargetMissing { .. } => "target_missing",
            Self::NativeQaMissing { .. } => "native_qa_missing",
            Self::NativeQaFailed { .. } => "native_qa_failed",
            Self::NotNative { .. } => "not_native",
            Self::CrossBuildRefused { .. } => "cross_build_refused",
            Self::ReceiptUnpinned { .. } => "target_receipt_invalid",
            Self::Unsealed { .. } => "artifact_unsealed",
            Self::UnknownTarget { .. } => "unknown_target",
            Self::ReleaseConflict { .. } => "release_conflict",
            Self::Tool { .. } => "tool_failed",
            Self::UnknownFixture { .. } => "unknown_fixture",
        }
    }

    /// Recovery guidance for the kind.
    #[must_use]
    pub(crate) const fn hint(&self) -> &'static str {
        match self {
            Self::Usage { .. } | Self::UnknownFixture { .. } => "run `release-stage --help`",
            Self::Io { .. } => "check the path and its permissions",
            Self::Malformed { .. } => {
                "regenerate the document with release-stage; never hand-edit it"
            }
            Self::ChecksumMismatch { .. } => {
                "rebuild and restage the artifact; never re-record a digest"
            }
            Self::VersionMismatch { .. } => {
                "bump the workspace and client-ts versions together and rebuild"
            }
            Self::SchemaMismatch => {
                "run `openrpc write schema/ovis-terminal.openrpc.json` and rebuild"
            }
            Self::CompatibilityBreak { .. } => {
                "restore the released method or required output; a break needs a new generation"
            }
            Self::BinaryFailed { .. } => {
                "run the command by hand on the native host and read its stderr"
            }
            Self::NoticesMissing { .. } => {
                "restore the Apache-2.0 attribution in THIRD_PARTY_NOTICES.md"
            }
            Self::NoticesIncomplete { .. } | Self::NoticesStale => {
                "run `release-stage notices --repo .` and commit THIRD_PARTY_NOTICES.md"
            }
            Self::TargetMissing { .. } => {
                "let every native target job finish; an absent artifact is never skipped"
            }
            Self::NativeQaMissing { .. } | Self::NativeQaFailed { .. } => {
                "run `release-stage qa` on the target's own runner and keep its receipt"
            }
            Self::NotNative { .. } | Self::CrossBuildRefused { .. } => {
                "build and check the target on its own OS and architecture"
            }
            Self::ReceiptUnpinned { .. } => {
                "use the toolchain scripts/release/pins.json pins, or change the pin in review"
            }
            Self::Unsealed { .. } => "run `release-stage seal` on the entry first",
            Self::UnknownTarget { .. } => {
                "use a target listed in scripts/release/pins.json, or `native`"
            }
            Self::ReleaseConflict { .. } => {
                "bump the version; a published version is never replaced"
            }
            Self::Tool { .. } => "install the named program on this host and rerun",
        }
    }

    /// 1 for a refused gate, 2 for usage, 3 for unreadable input.
    #[must_use]
    pub(crate) const fn exit(&self) -> u8 {
        match self {
            Self::Usage { .. } | Self::UnknownFixture { .. } => 2,
            Self::Io { .. } | Self::Tool { .. } => 3,
            _ => 1,
        }
    }

    /// The typed facts beyond kind and message.
    #[must_use]
    pub(crate) fn details(&self) -> Value {
        match self {
            Self::ChecksumMismatch {
                file,
                recorded,
                actual,
            } => {
                json!({ "file": file, "recorded": recorded, "actual": actual })
            }
            Self::CompatibilityBreak { binary } | Self::BinaryFailed { binary, .. } => {
                json!({ "binary": binary })
            }
            Self::NoticesIncomplete { crates } => json!({ "crates": crates }),
            Self::TargetMissing { targets } => json!({ "missing": targets }),
            Self::NativeQaFailed { target, failed } => {
                json!({ "target": target, "failed": failed })
            }
            Self::ReceiptUnpinned { field, .. } => json!({ "field": field }),
            Self::Tool { detail, .. } => json!({ "detail": detail }),
            _ => json!({}),
        }
    }

    pub(crate) fn io(
        operation: &'static str,
        path: &std::path::Path,
    ) -> impl FnOnce(io::Error) -> Self {
        let path = path.to_path_buf();
        move |source| Self::Io {
            operation,
            path,
            source,
        }
    }
}
