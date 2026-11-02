//! The cases: each runs literal `sh install.sh` command lines in its own world against the
//! local fixture releases and checks what they did to the install directory and around it.

use std::path::Path;

use serde_json::{Value, json};

use super::error::InstallProbeError;
use super::probe::{Expected, Probe};
use super::serve::Server;
use super::world::World;

/// The good release, latest.
pub(super) const INSTALL: &str = r#"OVIS_TERMINAL_INSTALL_DIR="$ATTEMPT_DIR/install" OVIS_TERMINAL_RELEASES_URL="$FIXTURE_RELEASES_URL" sh install.sh"#;
/// The good release, pinned to its version.
const PINNED: &str = r#"OVIS_TERMINAL_INSTALL_DIR="$ATTEMPT_DIR/install" OVIS_TERMINAL_VERSION="$FIXTURE_VERSION" OVIS_TERMINAL_RELEASES_URL="$FIXTURE_RELEASES_URL" sh install.sh"#;
/// The QA failure injection: one archive byte flipped, manifest and sidecar intact.
pub(super) const CORRUPT: &str = r#"OVIS_TERMINAL_INSTALL_DIR="$ATTEMPT_DIR/install" OVIS_TERMINAL_RELEASES_URL="$CORRUPT_RELEASES_URL" sh install.sh"#;
const TRAVERSAL: &str = r#"OVIS_TERMINAL_INSTALL_DIR="$ATTEMPT_DIR/install" OVIS_TERMINAL_RELEASES_URL="$TRAVERSAL_RELEASES_URL" sh install.sh"#;
const UNKNOWN: &str = r#"OVIS_TERMINAL_INSTALL_DIR="$ATTEMPT_DIR/install" OVIS_TERMINAL_RELEASES_URL="$UNKNOWN_TARGET_RELEASES_URL" sh install.sh"#;
const MISLABELED: &str = r#"OVIS_TERMINAL_INSTALL_DIR="$ATTEMPT_DIR/install" OVIS_TERMINAL_RELEASES_URL="$MISLABELED_RELEASES_URL" sh install.sh"#;
pub(super) const CHECK: &str =
    r#"OVIS_TERMINAL_INSTALL_DIR="$ATTEMPT_DIR/install" sh install.sh --check"#;
const UNINSTALL: &str =
    r#"OVIS_TERMINAL_INSTALL_DIR="$ATTEMPT_DIR/install" sh install.sh --uninstall"#;
const VERSION: &str = r#""$ATTEMPT_DIR/install/bin/ovis-terminal" --version"#;

/// Every case, in the order `--case all` runs them.
pub const CASES: [&str; 10] = [
    "happy",
    "checksum-mismatch",
    "traversal",
    "unknown-target",
    "version-mismatch",
    "unwritable",
    "interrupted-download",
    "running-runtime",
    "foreign-uninstall",
    "corruption",
];

/// What every case shares: scratch, checkout, fixture URLs, server and expectations.
pub(super) struct Context<'a> {
    pub(super) scratch: &'a Path,
    pub(super) checkout: &'a Path,
    pub(super) urls: &'a [(String, String)],
    pub(super) server: &'a Server,
    pub(super) expected: &'a Expected<'a>,
}

/// Runs `case` in a fresh world and returns its evidence object.
pub(super) fn run_case(ctx: &Context<'_>, case: &str) -> Result<Value, InstallProbeError> {
    let world = World::create(ctx.scratch, case, (ctx.checkout, ctx.urls))?;
    let mut probe = Probe::new(world, ctx.expected);
    match case {
        "happy" => happy(&mut probe)?,
        "checksum-mismatch" => refusal(&mut probe, (CORRUPT, 1, "checksum_mismatch"))?,
        "traversal" => traversal(&mut probe)?,
        "unknown-target" => refusal(&mut probe, (UNKNOWN, 4, "unknown_target"))?,
        "version-mismatch" => refusal(&mut probe, (MISLABELED, 1, "version_mismatch"))?,
        "unwritable" => super::adversarial::unwritable(&mut probe)?,
        "interrupted-download" => super::adversarial::interrupted(&mut probe, ctx.server)?,
        "running-runtime" => super::adversarial::running_runtime(&mut probe)?,
        "foreign-uninstall" => super::adversarial::foreign_uninstall(&mut probe)?,
        "corruption" => super::adversarial::corruption(&mut probe)?,
        other => {
            return Err(InstallProbeError::Usage {
                detail: format!("no case named {other}; cases: {}", CASES.join(", ")),
            });
        }
    }
    Ok(probe.finish(case))
}

/// Install, run, pin, verify and remove; the binary is the one under test and nothing
/// outside the install directory changes.
fn happy(probe: &mut Probe<'_>) -> Result<(), InstallProbeError> {
    let outside = probe.world.outside()?;
    let installed = probe.run(INSTALL)?;
    probe.expect(&installed, (0, "outcome", "installed"));
    probe.check_installed_is_under_test()?;
    let reported = installed
        .report
        .get("binary_sha256")
        .cloned()
        .unwrap_or(Value::Null);
    let expected = json!(probe.expected.binary_sha256);
    probe.check(
        "the report names the installed binary's hash",
        (reported == expected, reported),
    );
    let version = probe.run(VERSION)?;
    let line = format!("ovis-terminal {}", probe.expected.version);
    let matches = version.exit == Some(0) && version.stdout.trim() == line;
    probe.observe("installed_version", json!(version.stdout.trim()));
    probe.check(
        "the installed binary reports the release version",
        (matches, json!(version.stdout)),
    );
    let pinned = probe.run(PINNED)?;
    probe.expect(&pinned, (0, "outcome", "installed"));
    let checked = probe.run(CHECK)?;
    probe.expect(&checked, (0, "outcome", "verified"));
    probe.check_no_service()?;
    let removed = probe.run(UNINSTALL)?;
    probe.expect(&removed, (0, "outcome", "removed"));
    let gone = !probe.world.install().exists();
    probe.observe("install_dir_removed", json!(gone));
    probe.check(
        "removal leaves no install directory",
        (gone, json!(probe.world.install())),
    );
    let outside_changed = super::world::changed(&outside, &probe.world.outside()?);
    probe.observe("outside_unchanged", json!(outside_changed == json!([])));
    probe.check(
        "nothing outside the install directory changed",
        (outside_changed == json!([]), outside_changed),
    );
    Ok(())
}

/// A prior install, then `command` refused with `exit` and `kind`, then no runtime.
fn refusal(
    probe: &mut Probe<'_>,
    (command, exit, kind): (&str, i32, &str),
) -> Result<(), InstallProbeError> {
    probe.prepare(INSTALL)?;
    probe.refused(command, (exit, kind))?;
    probe.check_no_service()
}

/// The traversal archive is refused before anything is unpacked: no `escaped` file
/// appears anywhere in the world.
fn traversal(probe: &mut Probe<'_>) -> Result<(), InstallProbeError> {
    refusal(probe, (TRAVERSAL, 1, "unsafe_archive"))?;
    let escaped: Vec<String> = super::world::inventory(&probe.world.root, "world")?
        .into_keys()
        .filter(|path| path.ends_with("/escaped"))
        .collect();
    probe.observe("escaped_files", json!(escaped));
    probe.check(
        "no archive member escaped",
        (escaped.is_empty(), json!(escaped)),
    );
    Ok(())
}
