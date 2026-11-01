//! The committed QA manifests: every stress lane resolves, and the Windows smoke
//! manifest names every required check with tests that exist in their packages.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use ovis_terminal_probes::platform_qa::{SmokeManifest, lanes};

/// Checks the acceptance command of todo 60 requires from the Windows job.
const REQUIRED: [&str; 11] = [
    "named-pipe-hello",
    "conpty-run",
    "conpty-attach",
    "conpty-input",
    "conpty-resize",
    "final-output-order",
    "observer-denial",
    "two-clients",
    "core-restart",
    "job-survival",
    "conpty-kill",
];

/// Lanes the dispatch workflow carried as per-lane jobs before it became data-driven,
/// plus the platform QA lane. New lanes only add files, so this list is a floor.
const LANES: [&str; 43] = [
    "agent-status",
    "agent-tree",
    "boundary-policy",
    "cli",
    "client-ts",
    "commit-barrier",
    "conformance",
    "connection",
    "conpty",
    "conversation-records",
    "credential-rotation",
    "docs-retention",
    "encoding",
    "geometry",
    "graphics-caps",
    "health",
    "holder-shutdown",
    "input",
    "layout",
    "leases",
    "machine-actor",
    "machine-identity",
    "ops-admission",
    "ops-cancel",
    "ops-digest",
    "platform",
    "platform-qa",
    "plugins",
    "process-info",
    "prompt-bridge",
    "prompts",
    "quality-gates",
    "recovery",
    "restart",
    "screen",
    "service-integration",
    "session-binding",
    "subscriptions",
    "template-launch",
    "unix-service",
    "update",
    "waits",
    "windows-service",
];

fn here() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("platform-qa")
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            sources(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

fn package_dir(package: &str) -> PathBuf {
    let probes = Path::new(env!("CARGO_MANIFEST_DIR"));
    if package == "ovis-terminal-probes" {
        probes.to_path_buf()
    } else {
        probes.join("..").join("crates").join(package)
    }
}

#[test]
fn every_stress_lane_resolves_and_the_union_is_kept() {
    let found = lanes(&here().join("stress")).unwrap();
    let names: BTreeSet<&str> = found.keys().map(String::as_str).collect();
    let missing: Vec<&str> = LANES
        .into_iter()
        .filter(|lane| !names.contains(lane))
        .collect();
    assert_eq!(missing, Vec::<&str>::new());
    for name in &names {
        let script =
            std::fs::read_to_string(here().join("stress").join(format!("{name}.sh"))).unwrap();
        assert!(script.contains("set -euo pipefail"), "{name}");
        assert!(script.contains("RESULT "), "{name} never reports a result");
    }
}

#[test]
fn the_windows_smoke_manifest_names_every_required_check_with_existing_tests() {
    let manifest = SmokeManifest::load(&here().join("windows-smoke.json")).unwrap();
    let names: BTreeSet<&str> = manifest.checks.iter().map(|c| c.name.as_str()).collect();
    for required in REQUIRED {
        assert!(names.contains(required), "{required} is not a smoke check");
    }
    for test in manifest.checks.iter().flat_map(|check| &check.tests) {
        let mut files = Vec::new();
        sources(&package_dir(&test.package), &mut files).unwrap();
        let leaf = test.test.rsplit("::").next().unwrap();
        let declared = files.iter().any(|file| {
            std::fs::read_to_string(file)
                .unwrap()
                .contains(&format!("fn {leaf}("))
        });
        assert!(declared, "{} names no test in {}", test.test, test.package);
    }
}
