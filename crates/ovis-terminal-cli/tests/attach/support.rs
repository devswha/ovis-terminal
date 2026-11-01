//! A running `serve` with one shell, and an outer PTY driving one attach client.

#[path = "outer.rs"]
mod outer;

use std::io::{BufRead as _, BufReader};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use outer::Outer;
use serde_json::{Value, json};

/// Result of a test step; any failure fails the test.
pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The binary under test.
pub const BIN: &str = env!("CARGO_BIN_EXE_ovis-terminal");
/// Bound on every awaited outcome.
pub const WAIT: Duration = Duration::from_secs(15);
static NEXT: AtomicU32 = AtomicU32::new(0);

fn op_id() -> TestResult<String> {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let unique = u128::from(NEXT.fetch_add(1, Ordering::Relaxed));
    let id = (millis << 80)
        | (0x7 << 76)
        | (0b10 << 62)
        | (u128::from(std::process::id()) << 16)
        | unique;
    let hex = format!("{id:032x}");
    let part = |from: usize, to: usize| hex.get(from..to).unwrap_or_default().to_owned();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        part(0, 8),
        part(8, 12),
        part(12, 16),
        part(16, 20),
        part(20, 32)
    ))
}

/// A running `serve` with one `/bin/sh` launched as `t1`.
#[derive(Debug)]
pub struct Served {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    root: PathBuf,
    env: Vec<(&'static str, String)>,
}

impl Served {
    /// Starts `serve` in a fresh root and launches `/bin/sh` as `t1`.
    ///
    /// # Panics
    ///
    /// Panics when the shell does not spawn.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    pub fn start() -> TestResult<Self> {
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = PathBuf::from("/tmp").join(format!("ovt31b-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&root)?;
        let mut child = Command::new(BIN)
            .args(["serve", "--stdin-lifetime", "--root"])
            .arg(&root)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take();
        let mut stdout = BufReader::new(child.stdout.take().ok_or("stdout")?);
        let mut line = String::new();
        stdout.read_line(&mut line)?;
        let ready: Value = serde_json::from_str(&line)?;
        let field = |name: &str| {
            ready
                .pointer(&format!("/ready/{name}"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let env = vec![
            ("PATH", "/usr/bin:/bin".to_owned()),
            ("OVIS_TERMINAL_SOCKET", field("socket").ok_or("socket")?),
            ("OVIS_TERMINAL_CREDENTIAL", field("bearer").ok_or("bearer")?),
        ];
        let served = Self {
            child,
            stdin,
            stdout,
            root,
            env,
        };
        let launch = json!({
            "op_id": op_id()?, "program": {"argv": ["/bin/sh"]}, "geometry": {"cols": 80, "rows": 24},
            "cwd": "/tmp", "env": {"PATH": "/usr/bin:/bin", "PS1": "$ ", "TERM": "xterm"},
            "name": null, "labels": null, "parent": null, "child_policy": null, "keep": null,
            "first_message": null, "machine": null,
        });
        let run = served.call("terminal.run", &launch)?;
        assert_eq!(run.get("spawn"), Some(&json!("spawned")), "{run}");
        Ok(served)
    }

    /// The binary with only this server's endpoint, bearer and `PATH`.
    pub fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(BIN);
        command
            .args(args)
            .env_clear()
            .envs(self.env.iter().map(|(key, value)| (*key, value)));
        command
    }

    /// Sends one method through `call` and returns its parsed result.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    ///
    /// # Panics
    ///
    /// Panics when the binary's behaviour differs from the expectation.
    pub fn call(&self, method: &str, params: &Value) -> TestResult<Value> {
        let output = self
            .command(&["--json", "call", method, "--params", &params.to_string()])
            .output()?;
        assert_eq!(output.status.code(), Some(0), "{method}: {output:?}");
        Ok(serde_json::from_slice(&output.stdout)?)
    }

    /// The endpoint socket path and the owner bearer this server announced.
    ///
    /// # Errors
    ///
    /// Fails when either was missing from the ready line.
    pub fn endpoint(&self) -> TestResult<(String, String)> {
        let field = |name: &str| {
            self.env
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone())
                .ok_or_else(|| name.to_owned())
        };
        Ok((
            field("OVIS_TERMINAL_SOCKET")?,
            field("OVIS_TERMINAL_CREDENTIAL")?,
        ))
    }

    /// `attach.leases` of `t1`.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    pub fn leases(&self) -> TestResult<Value> {
        self.call("attach.leases", &json!({"target": {"address": "t1"}}))
    }

    /// Closes stdin: the server closes every connection, then exits 0.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    ///
    /// # Panics
    ///
    /// Panics when the binary's behaviour differs from the expectation.
    pub fn stop(mut self) -> TestResult {
        drop(self.stdin.take());
        let mut report = String::new();
        self.stdout.read_line(&mut report)?;
        assert_eq!(self.child.wait()?.code(), Some(0), "serve report {report}");
        std::fs::remove_dir_all(&self.root)?;
        Ok(())
    }
}
