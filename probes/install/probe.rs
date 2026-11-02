//! What one case ran and checked: literal commands, observations and pass/fail checks.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value, json};

use super::archive::sha256_file;
use super::error::InstallProbeError;
use super::run::{Ran, run};
use super::world::{World, changed, inventory};

/// `$ATTEMPT_DIR/install/bin/ovis-terminal` asked for its state, read-only.
pub(super) const STATUS: &str = r#""$ATTEMPT_DIR/install/bin/ovis-terminal" --json server status"#;

const BINARY_KEY: &str = "install/bin/ovis-terminal";

/// Facts every case compares the installed binary against.
pub(super) struct Expected<'a> {
    pub binary_sha256: &'a str,
    pub(super) version: &'a str,
}

/// One case in progress.
pub(super) struct Probe<'a> {
    pub(super) world: World,
    pub(super) expected: &'a Expected<'a>,
    commands: Vec<Value>,
    checks: Vec<Value>,
    observed: Map<String, Value>,
}

impl<'a> Probe<'a> {
    pub(super) fn new(world: World, expected: &'a Expected<'a>) -> Self {
        Self {
            world,
            expected,
            commands: Vec::new(),
            checks: Vec::new(),
            observed: Map::new(),
        }
    }

    /// Runs `command` literally and keeps it as evidence.
    pub(super) fn run(&mut self, command: &str) -> Result<Ran, InstallProbeError> {
        let ran = run(&self.world, command)?;
        self.record(&ran);
        Ok(ran)
    }

    pub(super) fn record(&mut self, ran: &Ran) {
        self.commands.push(ran.evidence());
    }

    pub(super) fn check(&mut self, check: &str, (passed, detail): (bool, Value)) {
        let mut record = Map::new();
        record.insert("check".to_owned(), json!(check));
        record.insert("passed".to_owned(), json!(passed));
        record.insert("detail".to_owned(), detail);
        self.checks.push(Value::Object(record));
    }

    pub(super) fn observe(&mut self, key: &str, value: Value) {
        self.observed.insert(key.to_owned(), value);
    }

    /// Requires `ran` to exit `exit` with `field` (`outcome` or `kind`) equal to `value`.
    pub(super) fn expect(&mut self, ran: &Ran, (exit, field, value): (i32, &str, &str)) {
        let found = if field == "kind" {
            ran.kind()
        } else {
            ran.outcome()
        };
        let passed = ran.exit == Some(exit) && found == value;
        self.check(
            &format!("{} -> exit {exit} {field} {value}", ran.command),
            (
                passed,
                json!({ "exit": ran.exit, field: found, "stderr": ran.stderr }),
            ),
        );
    }

    /// Installs the good release as the prior state a refusal must leave alone.
    pub(super) fn prepare(&mut self, install: &str) -> Result<(), InstallProbeError> {
        let ran = self.run(install)?;
        self.expect(&ran, (0, "outcome", "installed"));
        Ok(())
    }

    /// SHA-256 of the installed binary, or `absent`.
    pub(super) fn installed_sha256(&self) -> Result<String, InstallProbeError> {
        let binary = self.world.install().join("bin").join("ovis-terminal");
        if binary.is_file() {
            sha256_file(&binary)
        } else {
            Ok("absent".to_owned())
        }
    }

    /// Requires the installed binary to be byte for byte the binary under test.
    pub(super) fn check_installed_is_under_test(&mut self) -> Result<(), InstallProbeError> {
        let actual = self.installed_sha256()?;
        let expected = self.expected.binary_sha256.to_owned();
        self.observe("installed_binary_sha256", json!(actual));
        self.check(
            "the installed binary is the binary under test",
            (
                actual == expected,
                json!({ "expected": expected, "actual": actual }),
            ),
        );
        Ok(())
    }

    /// The state `server status` reports: a refusal must not have started a runtime.
    pub(super) fn check_no_service(&mut self) -> Result<(), InstallProbeError> {
        let ran = self.run(STATUS)?;
        let state = ran
            .report
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        self.observe("service_started", json!(state != "stopped"));
        self.check(
            "no runtime or service was started",
            (state == "stopped", json!({ "state": state })),
        );
        Ok(())
    }

    /// Runs `command`, which must be refused with `exit` and `kind`, and checks that the
    /// install directory and everything outside it are byte for byte what they were.
    pub(super) fn refused(
        &mut self,
        command: &str,
        (exit, kind): (i32, &str),
    ) -> Result<Ran, InstallProbeError> {
        let install = self.world.install();
        let outside = self.world.outside()?;
        let tree = snapshot(&install)?;
        let ran = self.run(command)?;
        self.expect(&ran, (exit, "kind", kind));
        let outside_changed = changed(&outside, &self.world.outside()?);
        let after = snapshot(&install)?;
        let install_changed = changed(&tree, &after);
        let binary = (tree.get(BINARY_KEY), after.get(BINARY_KEY));
        let binary_unchanged = binary.0.is_some() && binary.0 == binary.1;
        self.observe("binary_unchanged", json!(binary_unchanged));
        self.observe("outside_unchanged", json!(outside_changed == json!([])));
        self.observe("install_unchanged", json!(install_changed == json!([])));
        self.check(
            "the prior binary hash is identical",
            (binary_unchanged, json!(binary)),
        );
        self.check(
            "nothing outside the install directory changed",
            (outside_changed == json!([]), outside_changed),
        );
        self.check(
            "the install directory is unchanged",
            (install_changed == json!([]), install_changed),
        );
        Ok(ran)
    }

    /// The case's evidence object.
    pub(super) fn finish(self, case: &str) -> Value {
        let failed = self
            .checks
            .iter()
            .filter(|check| check.get("passed") != Some(&json!(true)))
            .count();
        json!({
            "case": case,
            "passed": failed == 0,
            "failed": failed.to_string(),
            "checks_total": self.checks.len().to_string(),
            "commands": self.commands,
            "observed": self.observed,
            "checks": self.checks,
        })
    }
}

/// The install directory's inventory, or an empty one when it does not exist.
pub(super) fn snapshot(install: &Path) -> Result<BTreeMap<String, String>, InstallProbeError> {
    if install.is_dir() {
        inventory(install, "install")
    } else {
        Ok(BTreeMap::new())
    }
}
