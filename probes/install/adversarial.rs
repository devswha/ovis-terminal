//! The cases that change the world around the installer: read-only destinations, a
//! download cancelled mid-transfer, a live runtime, a foreign directory and a binary
//! corrupted after install.

use std::io::{BufRead as _, BufReader};
use std::path::Path;
use std::sync::mpsc::channel;
use std::time::Duration;

use serde_json::{Value, json};

use super::cases::{CHECK, INSTALL};
use super::error::InstallProbeError;
use super::probe::Probe;
use super::run::{finish, signal_group};
use super::serve::Server;

const LOCKED: &str = r#"OVIS_TERMINAL_INSTALL_DIR="$ATTEMPT_DIR/locked/install" OVIS_TERMINAL_RELEASES_URL="$FIXTURE_RELEASES_URL" sh install.sh"#;
const STALL: &str = r#"OVIS_TERMINAL_INSTALL_DIR="$ATTEMPT_DIR/install" OVIS_TERMINAL_RELEASES_URL="$STALL_RELEASES_URL" sh install.sh"#;
const SERVE: &str = r#"exec "$ATTEMPT_DIR/install/bin/ovis-terminal" serve --stdin-lifetime"#;
const FOREIGN: &str =
    r#"OVIS_TERMINAL_INSTALL_DIR="$ATTEMPT_DIR/outside" sh install.sh --uninstall"#;

/// How long a live process may take to reach an awaited state.
const EVENT_BOUND: Duration = Duration::from_secs(60);

#[cfg(unix)]
fn read_only(dir: &Path, on: bool) -> Result<(), InstallProbeError> {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = if on { 0o555 } else { 0o755 };
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode))
        .map_err(InstallProbeError::io("chmod", dir))
}

#[cfg(not(unix))]
fn read_only(dir: &Path, on: bool) -> Result<(), InstallProbeError> {
    let mut permissions = std::fs::metadata(dir)
        .map_err(InstallProbeError::io("stat", dir))?
        .permissions();
    permissions.set_readonly(on);
    std::fs::set_permissions(dir, permissions).map_err(InstallProbeError::io("chmod", dir))
}

/// A read-only install directory and a read-only parent are both refused before any
/// download, and the installed binary stays.
pub(super) fn unwritable(probe: &mut Probe<'_>) -> Result<(), InstallProbeError> {
    probe.prepare(INSTALL)?;
    let install = probe.world.install();
    let locked = probe.world.attempt.join("locked");
    std::fs::create_dir_all(&locked).map_err(InstallProbeError::io("create", &locked))?;
    read_only(&install, true)?;
    read_only(&locked, true)?;
    let refused = probe.refused(INSTALL, (3, "unwritable_destination"));
    let fresh = refused.and_then(|_| probe.run(LOCKED));
    read_only(&install, false)?;
    read_only(&locked, false)?;
    let fresh = fresh?;
    probe.expect(&fresh, (3, "kind", "unwritable_destination"));
    let created = locked.join("install").exists();
    probe.check(
        "a read-only parent gets no install directory",
        (!created, json!(created)),
    );
    probe.check_no_service()
}

/// A download cancelled after half the archive arrived leaves the prior install, and
/// no workspace; the next run installs.
pub(super) fn interrupted(probe: &mut Probe<'_>, server: &Server) -> Result<(), InstallProbeError> {
    probe.prepare(INSTALL)?;
    let before = probe.installed_sha256()?;
    let tree = super::probe::snapshot(&probe.world.install())?;
    let child = probe
        .world
        .command(STALL)
        .spawn()
        .map_err(|source| InstallProbeError::Spawn {
            program: STALL.to_owned(),
            source,
        })?;
    let pid = child.id();
    let stalled = server.await_stall(EVENT_BOUND);
    let signalled = stalled.and_then(|()| signal_group(pid, "TERM"));
    server.release_stall();
    let cancelled = finish(child, STALL)?;
    signalled?;
    probe.record(&cancelled);
    probe.check(
        "the cancelled installer exits through its TERM trap",
        (
            cancelled.exit == Some(143),
            json!({ "exit": cancelled.exit, "stderr": cancelled.stderr }),
        ),
    );
    let after = probe.installed_sha256()?;
    probe.observe("binary_unchanged", json!(before == after));
    probe.check(
        "the prior binary hash is identical",
        (before == after, json!([before, after])),
    );
    let left = super::world::changed(&tree, &super::probe::snapshot(&probe.world.install())?);
    probe.observe("install_unchanged", json!(left == json!([])));
    probe.check(
        "no partial download or workspace is left",
        (left == json!([]), left),
    );
    let resumed = probe.run(INSTALL)?;
    probe.expect(&resumed, (0, "outcome", "installed"));
    probe.check_installed_is_under_test()?;
    probe.check_no_service()
}

fn ready_line(child: &mut std::process::Child) -> Result<String, InstallProbeError> {
    let stdout = child.stdout.take().ok_or(InstallProbeError::HangGuard {
        what: "the runtime's ready line",
        bound_ms: 0,
    })?;
    let (sent, line) = channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut first = String::new();
        let read = reader.read_line(&mut first).map(|_| first);
        // The probe stops listening only after its hang guard fired.
        if sent.send(read).is_err() {
            tracing::debug!("the ready line arrived after its hang guard");
        }
        // Keeps the pipe open until the runtime exits, so its last report is not EPIPE.
        if let Err(error) = std::io::copy(&mut reader, &mut std::io::sink()) {
            tracing::debug!(%error, "the runtime's stdout ended with an error");
        }
    });
    line.recv_timeout(EVENT_BOUND)
        .map_err(|_| InstallProbeError::HangGuard {
            what: "the runtime's ready line",
            bound_ms: u64::try_from(EVENT_BOUND.as_millis()).unwrap_or(u64::MAX),
        })?
        .map_err(|source| InstallProbeError::Spawn {
            program: SERVE.to_owned(),
            source,
        })
}

/// With the installed runtime serving, the installer keeps its binary and hands the
/// verified release to the staged update; the runtime keeps running until it is stopped.
pub(super) fn running_runtime(probe: &mut Probe<'_>) -> Result<(), InstallProbeError> {
    probe.prepare(INSTALL)?;
    let mut serve = probe.world.spawn_held(SERVE)?;
    let ready = ready_line(&mut serve);
    let refused = if ready.is_ok() {
        Some(probe.refused(INSTALL, (5, "runtime_running")))
    } else {
        None
    };
    drop(serve.stdin.take());
    let stopped = finish(serve, SERVE)?;
    probe.record(&stopped);
    let ready = ready?;
    let refused = refused.transpose()?;
    let delegate = refused
        .as_ref()
        .and_then(|ran| ran.report.get("delegate").cloned())
        .unwrap_or(Value::Null);
    probe.observe("ready", json!(ready.contains("\"ready\"")));
    probe.observe("delegate", delegate.clone());
    probe.check(
        "the runtime announced itself",
        (ready.contains("\"ready\""), json!(ready.len())),
    );
    probe.check(
        "the installer delegates to update.stage",
        (delegate == json!("update.stage"), delegate),
    );
    probe.check(
        "the runtime ran until stdin closed",
        (stopped.exit == Some(0), json!(stopped.exit)),
    );
    probe.check_no_service()
}

/// `--uninstall` of a directory without an install receipt removes nothing.
pub(super) fn foreign_uninstall(probe: &mut Probe<'_>) -> Result<(), InstallProbeError> {
    let outside = probe.world.outside()?;
    let ran = probe.run(FOREIGN)?;
    probe.expect(&ran, (3, "kind", "not_installed"));
    let changed = super::world::changed(&outside, &probe.world.outside()?);
    probe.observe("outside_unchanged", json!(changed == json!([])));
    probe.check(
        "a foreign directory keeps every file",
        (changed == json!([]), changed),
    );
    Ok(())
}

/// A binary changed after install fails `--check`; reinstalling repairs it.
pub(super) fn corruption(probe: &mut Probe<'_>) -> Result<(), InstallProbeError> {
    probe.prepare(INSTALL)?;
    let binary = probe.world.install().join("bin").join("ovis-terminal");
    let mut bytes = std::fs::read(&binary).map_err(InstallProbeError::io("read", &binary))?;
    bytes.push(0);
    std::fs::write(&binary, bytes).map_err(InstallProbeError::io("write", &binary))?;
    let diagnosed = probe.run(CHECK)?;
    probe.expect(&diagnosed, (1, "kind", "checksum_mismatch"));
    let repaired = probe.run(INSTALL)?;
    probe.expect(&repaired, (0, "outcome", "installed"));
    let verified = probe.run(CHECK)?;
    probe.expect(&verified, (0, "outcome", "verified"));
    probe.check_installed_is_under_test()?;
    probe.check_no_service()
}
