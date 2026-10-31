//! An install root, a release source with real ustar archives, and a control that records
//! restarts instead of replacing a server.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ovis_terminal_protocol::domain::update::{BuildTarget, HolderGeneration, ReleaseVersion};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::{ServerBootId, UpdateId};
use ovis_terminal_protocol::schema::methods::server::ServerTransitionResult;
use ovis_terminal_protocol::schema::methods::update::{UpdateStageParams, UpdateStepParams};
use ovis_terminal_protocol::schema::scalar::{Incarnation, ProtocolGeneration};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use tokio::runtime::Runtime;
use uuid::Uuid;

use crate::update::{
    Config, CurrentBuild, Limits, Requirement, RestartControl, RestartFailure, Updates,
};

pub(super) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub(super) const LIVE: [u64; 2] = [3, 7];

/// Counts restarts; refuses them while `refuse` is set. Live incarnations never change.
#[derive(Debug, Default)]
pub(super) struct Control {
    pub(super) restarts: AtomicUsize,
    pub(super) refuse: Mutex<bool>,
}

#[derive(Debug, thiserror::Error)]
#[error("the fixture control refused the restart")]
struct Refused;

impl RestartControl for Control {
    fn restart(&self, _: &OpId) -> Result<ServerTransitionResult, RestartFailure> {
        if self.refuse.lock().map_or(true, |refuse| *refuse) {
            return Err(RestartFailure::Control {
                kind: "restarting",
                hint: "retry after the restart finishes",
                source: Box::new(Refused),
            });
        }
        let boot = self.restarts.fetch_add(1, Ordering::SeqCst);
        let old = ServerBootId::new(format!("boot-{boot}"));
        let new = ServerBootId::new(format!("boot-{}", boot.saturating_add(1)));
        Ok(ServerTransitionResult::restarted(
            (old, new),
            LIVE.map(Incarnation::new).to_vec(),
            Vec::new(),
        ))
    }

    fn live_incarnations(&self) -> Result<Vec<Incarnation>, RestartFailure> {
        Ok(LIVE.map(Incarnation::new).to_vec())
    }
}

/// One entry of a hand-built ustar archive.
pub(super) struct Entry<'a> {
    pub(super) name: &'a str,
    pub(super) kind: u8,
    pub(super) body: &'a [u8],
}

fn padded(text: &[u8], len: usize) -> Vec<u8> {
    let mut field = text.to_vec();
    field.resize(len, 0);
    field
}

fn octal(value: u64, len: usize) -> Vec<u8> {
    format!("{value:0width$o}\0", width = len.saturating_sub(1)).into_bytes()
}

/// A POSIX ustar archive of `entries`, ended by two zero blocks.
pub(super) fn tar(entries: &[Entry<'_>]) -> Vec<u8> {
    let mut out = Vec::new();
    for entry in entries {
        let size = u64::try_from(entry.body.len()).unwrap_or(u64::MAX);
        let mut header = [
            padded(entry.name.as_bytes(), 100),
            octal(0o755, 8),
            octal(0, 8),
            octal(0, 8),
            octal(size, 12),
            octal(0, 12),
            b"        ".to_vec(),
            vec![entry.kind],
            padded(b"", 100),
            b"ustar\0".to_vec(),
            b"00".to_vec(),
            padded(b"", 80),
            padded(b"", 167),
        ]
        .concat();
        let sum: u64 = header.iter().map(|byte| u64::from(*byte)).sum();
        header.splice(148..156, format!("{sum:06o}\0 ").into_bytes());
        out.extend_from_slice(&header);
        out.extend_from_slice(entry.body);
        out.resize(out.len().next_multiple_of(512), 0);
    }
    out.resize(out.len().saturating_add(1024), 0);
    out
}

pub(super) fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect()
}

/// A script that answers `--json --version` like the real CLI, or exits with `status`.
pub(super) fn script(version: &str, status: u8) -> Vec<u8> {
    let report = json!({"binary": "ovis-terminal", "version": version, "generation": "1"});
    format!("#!/bin/sh\nprintf '%s\\n' '{report}'\nexit {status}\n").into_bytes()
}

pub(super) fn manifest(version: &str, target: &str, holder: &str) -> Vec<u8> {
    json!({
        "version": version,
        "target": target,
        "protocol_generations": ["1"],
        "holder_generations": [holder],
    })
    .to_string()
    .into_bytes()
}

/// A release whose archive holds `build.json` and an executable answering readiness.
pub(super) fn release(version: &str, status: u8) -> Vec<u8> {
    let target = BuildTarget::host();
    let manifest = manifest(version, target.as_str(), "2");
    let binary = script(version, status);
    tar(&[
        Entry {
            name: "build.json",
            kind: b'0',
            body: &manifest,
        },
        Entry {
            name: "ovis-terminal",
            kind: b'0',
            body: &binary,
        },
    ])
}

pub(super) struct Fixture {
    pub(super) root: PathBuf,
    pub(super) control: Arc<Control>,
    pub(super) runtime: Runtime,
}

impl Fixture {
    pub(super) fn new() -> TestResult<Self> {
        let root = std::env::temp_dir().join(format!("ovis-t47-{}", Uuid::now_v7()));
        std::fs::create_dir_all(root.join("current"))?;
        std::fs::write(root.join("current/ovis-terminal"), script("1.0.0", 0))?;
        Ok(Self {
            root,
            control: Arc::new(Control::default()),
            runtime: Runtime::new()?,
        })
    }

    pub(super) fn current_binary(&self) -> PathBuf {
        self.root.join("current/ovis-terminal")
    }

    pub(super) fn install_root(&self) -> PathBuf {
        self.root.join("install")
    }

    /// Publishes `archive` as `version` for `target` in the release source.
    pub(super) fn publish(
        &self,
        (version, target): (&str, &str),
        archive: &[u8],
    ) -> TestResult<String> {
        let dir = self.root.join("releases").join(version);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(
            dir.join(format!("ovis-terminal-{version}-{target}.tar")),
            archive,
        )?;
        Ok(sha256(archive))
    }

    pub(super) fn open(&self) -> TestResult<Updates> {
        let config = Config::new(
            (self.install_root(), self.root.join("releases")),
            CurrentBuild::new("1.0.0".parse()?, self.current_binary()),
            Requirement::new(ProtocolGeneration::new(1), HolderGeneration::new(2)),
        )
        .with_limits(Limits::new(1024 * 1024, Duration::from_secs(10)));
        let _guard = self.runtime.enter();
        Ok(Updates::open(
            config,
            Arc::clone(&self.control) as Arc<dyn RestartControl>,
        )?)
    }

    /// Runs a step with this fixture's runtime current, as the dispatcher's blocking pool does.
    pub(super) fn step<T>(&self, step: impl FnOnce() -> T) -> T {
        let _guard = self.runtime.enter();
        step()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.root);
        assert!(removed.is_ok(), "remove owned fixture: {removed:?}");
    }
}

pub(super) fn stage_params(version: &str, sha256: &str) -> TestResult<UpdateStageParams> {
    Ok(serde_json::from_value(json!({
        "op_id": format!("op-stage-{}", Uuid::now_v7()),
        "version": version,
        "target": BuildTarget::host().as_str(),
        "sha256": sha256,
    }))?)
}

pub(super) fn step_params(id: &UpdateId) -> TestResult<UpdateStepParams> {
    Ok(serde_json::from_value(json!({
        "op_id": format!("op-step-{}", Uuid::now_v7()),
        "update_id": id.as_str(),
    }))?)
}

pub(super) fn version(text: &str) -> TestResult<ReleaseVersion> {
    Ok(text.parse()?)
}

pub(super) fn exists_below(dir: &Path, name: &str) -> bool {
    walk(dir)
        .iter()
        .any(|path| path.file_name().is_some_and(|file| file == name))
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .flat_map(|entry| {
            let path = entry.path();
            let mut found = walk(&path);
            found.push(path);
            found
        })
        .collect()
}
