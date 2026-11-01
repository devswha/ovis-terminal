//! Hardware, build and binary-under-test facts every report states explicitly.

use std::path::Path;
use std::process::Command;

use serde::Serialize;
use sha2::{Digest as _, Sha256};

use super::error::Terminal50Error;

#[derive(Debug, Clone, Serialize)]
pub(super) struct Host {
    pub(super) os: &'static str,
    pub(super) arch: &'static str,
    pub(super) kernel: String,
    pub(super) cpu_model: String,
    pub(super) available_parallelism: usize,
    pub(super) runtime_workers: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Build {
    pub(super) bench_version: &'static str,
    pub(super) profile: &'static str,
    pub(super) debug_assertions: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Binary {
    pub(super) path: String,
    pub(super) sha256: String,
    pub(super) bytes: u64,
    pub(super) version: String,
}

fn command_line(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map_or_else(
            || format!("unavailable: {program} {}", args.join(" ")),
            |output| String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        )
}

fn cpu_model() -> String {
    if cfg!(target_os = "macos") {
        return command_line("sysctl", &["-n", "machdep.cpu.brand_string"]);
    }
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|info| {
            info.lines()
                .find_map(|line| line.strip_prefix("model name"))
                .map(|rest| rest.trim_start_matches([' ', '\t', ':']).to_owned())
        })
        .unwrap_or_else(|| "unavailable: /proc/cpuinfo has no model name".to_owned())
}

pub(super) fn host(runtime_workers: usize) -> Host {
    Host {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        kernel: command_line("uname", &["-srm"]),
        cpu_model: cpu_model(),
        available_parallelism: std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get),
        runtime_workers,
    }
}

pub(super) const fn build() -> Build {
    Build {
        bench_version: env!("CARGO_PKG_VERSION"),
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        debug_assertions: cfg!(debug_assertions),
    }
}

/// Hash, size and `--version` of the binary under test.
pub(super) fn binary(path: &Path) -> Result<Binary, Terminal50Error> {
    let bytes = std::fs::read(path).map_err(|source| Terminal50Error::Binary { source })?;
    let version = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|source| Terminal50Error::Binary { source })?;
    Ok(Binary {
        path: path.display().to_string(),
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        version: String::from_utf8_lossy(&version.stdout).trim().to_owned(),
    })
}
