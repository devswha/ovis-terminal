//! `update.*` against a real `serve`: a release published below the state directory is
//! staged by digest, verified by its readiness probe and activated, which restarts control
//! onto it; the successor boot serves the same program.

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use super::restart::{closing, pid};
use super::support::{Served, TestResult, op_id};

const VERSION: &str = "9.9.9";

fn call(served: &Served, method: &str, params: &Value) -> TestResult<Value> {
    served.ok(&["call", method, "--params", &params.to_string()])
}

/// Archives a build of `VERSION` whose readiness probe answers like the binary, below
/// `releases`; returns the archive's SHA-256.
fn publish(releases: &Path) -> TestResult<String> {
    let build = releases.join("build");
    std::fs::create_dir_all(&build)?;
    let report = json!({"binary": "ovis-terminal", "version": VERSION, "generation": "1"});
    let script = build.join("ovis-terminal");
    std::fs::write(&script, format!("#!/bin/sh\nprintf '%s\\n' '{report}'\n"))?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
    let target = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let manifest = json!({"version": VERSION, "target": target,
        "protocol_generations": ["1"], "holder_generations": ["2"]});
    std::fs::write(build.join("build.json"), manifest.to_string())?;
    let dir = releases.join(VERSION);
    std::fs::create_dir_all(&dir)?;
    let archive = dir.join(format!("ovis-terminal-{VERSION}-{target}.tar"));
    let tar = Command::new("tar")
        .args(["--format=ustar", "-cf"])
        .arg(&archive)
        .arg("-C")
        .arg(&build)
        .args(["build.json", "ovis-terminal"])
        .output()?;
    assert!(tar.status.success(), "{tar:?}");
    let digest = Command::new("shasum")
        .args(["-a", "256"])
        .arg(&archive)
        .output()?;
    assert!(digest.status.success(), "{digest:?}");
    let text = String::from_utf8(digest.stdout)?;
    Ok(text
        .split_whitespace()
        .next()
        .ok_or("no digest")?
        .to_owned())
}

#[test]
fn an_activated_update_restarts_control_onto_the_staged_build_and_keeps_the_program() -> TestResult
{
    let mut served = Served::start()?;
    served.run_shell("shell")?;
    let query = call(&served, "update.query", &json!({}))?;
    assert_eq!(query.get("updates"), Some(&json!([])), "{query}");
    let sha256 = publish(&served.root().join("state").join("updates").join("releases"))?;
    let staged = call(
        &served,
        "update.stage",
        &json!({"op_id": op_id()?, "version": VERSION, "target": null, "sha256": sha256}),
    )?;
    let update = staged
        .get("update_id")
        .cloned()
        .ok_or_else(|| staged.to_string())?;
    let verified = call(
        &served,
        "update.verify",
        &json!({"op_id": op_id()?, "update_id": update}),
    )?;
    assert_eq!(verified.get("update_id"), Some(&update), "{verified}");
    let program = pid(&served)?;
    let before = served.hello_reply(served.bearer())?;
    let mut held = served.connected()?;
    let activated = call(
        &served,
        "update.activate",
        &json!({"op_id": op_id()?, "update_id": update}),
    )?;
    assert_eq!(activated.get("update_id"), Some(&update), "{activated}");
    let closed = closing(&mut held)?;
    assert_eq!(
        closed.pointer("/params/reason"),
        Some(&json!("server_restart"))
    );
    let announced = served.announcement()?;
    let successor = announced
        .pointer("/restarted/server_boot_id")
        .cloned()
        .ok_or_else(|| announced.to_string())?;
    assert_ne!(Some(&successor), before.pointer("/result/server_boot_id"));
    let after = served.hello_reply(served.bearer())?;
    assert_eq!(after.pointer("/result/server_boot_id"), Some(&successor));
    assert_eq!(
        after.pointer("/result/runtime_lifetime"),
        before.pointer("/result/runtime_lifetime")
    );
    assert_eq!(pid(&served)?, program, "the activation kept the program");
    let query = call(&served, "update.query", &json!({}))?;
    assert_eq!(
        query.get("active_version"),
        Some(&json!(VERSION)),
        "{query}"
    );
    let report = served.stop()?;
    assert_eq!(report.pointer("/report/panicked"), Some(&json!("0")));
    Ok(())
}
