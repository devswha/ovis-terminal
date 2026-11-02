//! Running one literal command line and keeping what it did as evidence.

use std::process::{Child, Output};
use std::sync::mpsc::channel;
use std::time::Duration;

use serde_json::{Value, json};

use super::error::InstallProbeError;
use super::world::World;

/// The longest any single installer command may take before the probe stops it.
pub(super) const RUN_BOUND: Duration = Duration::from_secs(300);

/// One finished command: its literal text, exit status, output and parsed report.
#[derive(Debug, Clone)]
pub(super) struct Ran {
    pub(super) command: String,
    pub(super) exit: Option<i32>,
    pub(super) stdout: String,
    pub(super) stderr: String,
    /// The last stdout line parsed as JSON, or null.
    pub(super) report: Value,
}

impl Ran {
    pub(super) fn evidence(&self) -> Value {
        json!({
            "command": self.command,
            "exit": self.exit,
            "stdout": self.stdout,
            "stderr": self.stderr,
            "report": self.report,
        })
    }

    pub(super) fn kind(&self) -> &str {
        self.report
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    pub(super) fn outcome(&self) -> &str {
        self.report
            .get("outcome")
            .and_then(Value::as_str)
            .unwrap_or("")
    }
}

fn ran(command: &str, output: &Output) -> Ran {
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let report = stdout
        .lines()
        .last()
        .and_then(|line| serde_json::from_str(line).ok())
        .unwrap_or(Value::Null);
    Ran {
        command: command.to_owned(),
        exit: output.status.code().or_else(|| signalled(output.status)),
        stdout,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        report,
    }
}

/// A shell's view of a signal death: 128 + the signal number.
#[cfg(unix)]
fn signalled(status: std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt as _;
    status.signal().map(|signal| signal.saturating_add(128))
}

#[cfg(not(unix))]
const fn signalled(_status: std::process::ExitStatus) -> Option<i32> {
    None
}

/// Sends `signal` to the process group the probe created for `pid`.
pub(super) fn signal_group(pid: u32, signal: &str) -> Result<(), InstallProbeError> {
    if pid <= 1 {
        return Err(InstallProbeError::Usage {
            detail: format!("refusing to signal process group {pid}"),
        });
    }
    let status = std::process::Command::new("kill")
        .arg(format!("-{signal}"))
        .arg("--")
        .arg(format!("-{pid}"))
        .status()
        .map_err(|source| InstallProbeError::Spawn {
            program: format!("kill -{signal} -- -{pid}"),
            source,
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(InstallProbeError::Spawn {
            program: format!("kill -{signal} -- -{pid}"),
            source: std::io::Error::other(format!("kill exited {status}")),
        })
    }
}

/// Waits for `child` with the hang guard; a child past it is killed with its group.
pub(super) fn finish(child: Child, command: &str) -> Result<Ran, InstallProbeError> {
    let pid = child.id();
    let (done, waited) = channel();
    std::thread::spawn(move || {
        // The probe stops listening only after the hang guard fired and killed the group.
        if done.send(child.wait_with_output()).is_err() {
            tracing::debug!("a command finished after its hang guard");
        }
    });
    let Ok(output) = waited.recv_timeout(RUN_BOUND) else {
        signal_group(pid, "KILL")?;
        return Err(InstallProbeError::HangGuard {
            what: "an installer command",
            bound_ms: u64::try_from(RUN_BOUND.as_millis()).unwrap_or(u64::MAX),
        });
    };
    let output = output.map_err(|source| InstallProbeError::Spawn {
        program: command.to_owned(),
        source,
    })?;
    Ok(ran(command, &output))
}

/// Runs `command` literally in `world` and waits for it.
pub(super) fn run(world: &World, command: &str) -> Result<Ran, InstallProbeError> {
    let child = world
        .command(command)
        .spawn()
        .map_err(|source| InstallProbeError::Spawn {
            program: command.to_owned(),
            source,
        })?;
    finish(child, command)
}
