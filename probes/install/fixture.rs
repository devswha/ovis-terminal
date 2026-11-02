//! The local fixture releases: one real artifact staged and sealed by `release-stage`
//! from the binary under test, published in the GitHub release layout, and the
//! adversarial copies derived from it.
//!
//! ```text
//! <scratch>/releases/<tree>/latest/download/{release-manifest.json,<archive>,<archive>.sha256}
//! <scratch>/releases/<tree>/download/v<version>/...
//! ```

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::archive::{Member, gzip_to, sha256_file, sha256_hex, ustar};
use super::error::InstallProbeError;

/// The version the mislabeled release claims while its binary reports the real one.
pub(super) const MISLABELED_VERSION: &str = "99.0.0";

/// Where the fixture comes from: the binary under test, the repository and scratch.
pub(super) struct Inputs<'a> {
    pub(super) binary: &'a Path,
    pub(super) repo: &'a Path,
    pub(super) scratch: &'a Path,
}

/// The published fixture trees and the facts the cases compare against.
#[derive(Debug, Clone)]
pub(super) struct Fixture {
    /// Served root holding one directory per tree.
    pub(super) root: PathBuf,
    pub(super) version: String,
    pub(super) archive: String,
    pub archive_sha256: String,
}

/// One target of a release manifest, in `release-stage promote`'s field names.
struct Target<'a> {
    triple: &'a str,
    platform: (&'a str, &'a str),
    archive: &'a str,
    bytes: &'a [u8],
}

fn manifest(version: &str, generation: &str, target: &Target<'_>) -> Value {
    json!({
        "format": "ovis-release-manifest/1",
        "version": version,
        "generation": generation,
        "signing": { "performed": false, "statement": "local fixture release; not signed" },
        "targets": [{
            "target": target.triple,
            "platform": { "os": target.platform.0, "arch": target.platform.1 },
            "archive": target.archive,
            "sha256": sha256_hex(target.bytes),
            "bytes": target.bytes.len().to_string(),
            "runner": "conformance-fixture",
            "qa_sha256": sha256_hex(b"fixture"),
        }],
    })
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), InstallProbeError> {
    std::fs::write(path, bytes).map_err(InstallProbeError::io("write", path))
}

/// Writes `files` into both download layouts of `tree`.
fn publish(
    root: &Path,
    (tree, version): (&str, &str),
    files: &[(&str, &[u8])],
) -> Result<(), InstallProbeError> {
    for layout in ["latest/download".to_owned(), format!("download/v{version}")] {
        let dir = root.join(tree).join(layout);
        std::fs::create_dir_all(&dir).map_err(InstallProbeError::io("create", &dir))?;
        for (name, bytes) in files {
            write(&dir.join(name), bytes)?;
        }
    }
    Ok(())
}

fn release(
    root: &Path,
    (tree, version, generation): (&str, &str, &str),
    target: &Target<'_>,
) -> Result<(), InstallProbeError> {
    let doc = manifest(version, generation, target);
    let text = serde_json::to_vec_pretty(&doc).map_err(|source| InstallProbeError::Json {
        what: "a fixture manifest",
        source,
    })?;
    let sidecar = format!("{}  {}\n", sha256_hex(target.bytes), target.archive);
    publish(
        root,
        (tree, version),
        &[
            ("release-manifest.json", &text),
            (target.archive, target.bytes),
            (&format!("{}.sha256", target.archive), sidecar.as_bytes()),
        ],
    )
}

fn str_at<'v>(value: &'v Value, pointer: &str) -> Result<&'v str, InstallProbeError> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| InstallProbeError::Fixture {
            message: format!("the native-artifact report has no {pointer}"),
        })
}

fn staged(inputs: &Inputs<'_>) -> Result<Value, InstallProbeError> {
    let path = |path: &Path| path.to_string_lossy().into_owned();
    let run = release_stage::execute(&[
        "fixture".to_owned(),
        "native-artifact".to_owned(),
        "--binary".to_owned(),
        path(inputs.binary),
        "--repo".to_owned(),
        path(inputs.repo),
        "--scratch".to_owned(),
        path(&inputs.scratch.join("stage")),
    ]);
    if run.exit != 0 {
        return Err(InstallProbeError::Fixture {
            message: run
                .report
                .get("message")
                .map(ToString::to_string)
                .unwrap_or_default(),
        });
    }
    Ok(run.report)
}

fn packed(
    scratch: &Path,
    label: &str,
    members: &[Member<'_>],
) -> Result<Vec<u8>, InstallProbeError> {
    let path = scratch.join(format!("{label}.tar.gz"));
    gzip_to(&ustar(members)?, &path)?;
    std::fs::read(&path).map_err(InstallProbeError::io("read", &path))
}

/// Stages the real artifact and publishes every fixture tree below `<scratch>/releases`.
pub(super) fn build(inputs: &Inputs<'_>) -> Result<Fixture, InstallProbeError> {
    let report = staged(inputs)?;
    let name = str_at(&report, "/entry/name")?;
    let version = str_at(&report, "/entry/version")?;
    let generation = str_at(&report, "/entry/generation")?;
    let triple = str_at(&report, "/entry/target")?;
    let archive = str_at(&report, "/entry/archive/file")?;
    let out = inputs
        .scratch
        .join("stage")
        .join("release-native-artifact")
        .join("out");
    let bytes = std::fs::read(out.join(archive)).map_err(InstallProbeError::io("read", &out))?;
    let binary =
        std::fs::read(inputs.binary).map_err(InstallProbeError::io("read", inputs.binary))?;
    let root = inputs.scratch.join("releases");
    let host = (std::env::consts::OS, std::env::consts::ARCH);
    let good = Target {
        triple,
        platform: host,
        archive,
        bytes: &bytes,
    };
    release(&root, ("good", version, generation), &good)?;
    let unknown = Target {
        platform: ("plan9", "mips64"),
        ..good
    };
    release(&root, ("unknown-target", version, generation), &unknown)?;
    let mut flipped = bytes.clone();
    if let Some(byte) = flipped.get_mut(bytes.len() / 2) {
        *byte ^= 0xff;
    }
    let sidecar = std::fs::read(out.join(format!("{archive}.sha256")))
        .map_err(InstallProbeError::io("read", &out))?;
    let manifest_text = std::fs::read(root.join("good/latest/download/release-manifest.json"))
        .map_err(InstallProbeError::io("read", &root))?;
    publish(
        &root,
        ("corrupt", version),
        &[
            ("release-manifest.json", &manifest_text),
            (archive, &flipped),
            (&format!("{archive}.sha256"), &sidecar),
        ],
    )?;
    let inside = format!("{name}/ovis-terminal");
    let escape = format!("{name}/../../escaped");
    let evil = packed(
        inputs.scratch,
        "traversal",
        &[
            Member {
                name: &inside,
                mode: 0o755,
                bytes: &binary,
            },
            Member {
                name: &escape,
                mode: 0o644,
                bytes: b"escaped the install directory\n",
            },
        ],
    )?;
    release(
        &root,
        ("traversal", version, generation),
        &Target {
            bytes: &evil,
            ..good
        },
    )?;
    let liar = name.replace(version, MISLABELED_VERSION);
    let liar_archive = format!("{liar}.tar.gz");
    let liar_inside = format!("{liar}/ovis-terminal");
    let lying = packed(
        inputs.scratch,
        "mislabeled",
        &[Member {
            name: &liar_inside,
            mode: 0o755,
            bytes: &binary,
        }],
    )?;
    let mislabeled = Target {
        archive: &liar_archive,
        bytes: &lying,
        ..good
    };
    release(
        &root,
        ("mislabeled", MISLABELED_VERSION, generation),
        &mislabeled,
    )?;
    Ok(Fixture {
        archive_sha256: sha256_file(&out.join(archive))?,
        root,
        version: version.to_owned(),
        archive: archive.to_owned(),
    })
}
