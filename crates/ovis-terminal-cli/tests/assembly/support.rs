//! A running `ovis-terminal serve` and the binary invoked against it.

use std::collections::BTreeSet;
use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// Result of a test step; any failure fails the test.
pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The binary under test.
pub const BIN: &str = env!("CARGO_BIN_EXE_ovis-terminal");
static NEXT: AtomicU32 = AtomicU32::new(0);

/// A fresh `UUIDv7` operation identity for the current time.
///
/// # Errors
///
/// Fails when a process, socket or file step fails.
pub fn op_id() -> TestResult<String> {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let unique = u128::from(NEXT.fetch_add(1, Ordering::Relaxed));
    let id = (millis << 80)
        | (0x7 << 76)
        | (0b10 << 62)
        | (u128::from(std::process::id()) << 16)
        | unique;
    Ok(uuid_text(id))
}

/// Where test state lives: `OVIS_FIXTURE_ROOT` when the host sets it, else `/tmp`, which
/// stays inside every platform's socket path limit.
fn fixture_root() -> PathBuf {
    std::env::var_os("OVIS_FIXTURE_ROOT").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from)
}

fn uuid_text(id: u128) -> String {
    let hex = format!("{id:032x}");
    let part = |from: usize, to: usize| hex.get(from..to).unwrap_or_default().to_owned();
    format!(
        "{}-{}-{}-{}-{}",
        part(0, 8),
        part(8, 12),
        part(12, 16),
        part(16, 20),
        part(20, 32)
    )
}

/// A running `ovis-terminal serve --root <dir> --stdin-lifetime`.
#[derive(Debug)]
pub struct Served {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    root: PathBuf,
    socket: String,
    bearer: String,
    observer: String,
    websocket: Option<String>,
    ready: Value,
}

impl Served {
    /// Starts `serve` with no extra flags.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    pub fn start() -> TestResult<Self> {
        Self::start_with(&[])
    }

    /// The WebSocket listener address from the ready line, when one was started.
    pub fn websocket(&self) -> Option<&str> {
        self.websocket.as_deref()
    }

    /// The observe-only bearer from the ready line.
    pub fn observer(&self) -> &str {
        &self.observer
    }

    /// The owner bearer from the ready line.
    pub fn bearer(&self) -> &str {
        &self.bearer
    }

    /// Starts `serve --root <fresh dir> --stdin-lifetime <extra>` and reads its ready line.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    pub fn start_with(extra: &[&str]) -> TestResult<Self> {
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = fixture_root().join(format!("ovt31a-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&root)?;
        Self::start_at(&root, extra)
    }

    /// Starts `serve` on an existing `root`, as a later boot of the same state.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    pub fn start_at(root: &Path, extra: &[&str]) -> TestResult<Self> {
        let root = root.to_path_buf();
        let mut child = Command::new(BIN)
            .args(["serve", "--root"])
            .arg(&root)
            .arg("--stdin-lifetime")
            .args(extra)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child.stdin.take();
        let mut stdout = BufReader::new(child.stdout.take().ok_or("stdout")?);
        let mut line = String::new();
        stdout.read_line(&mut line)?;
        let ready: Value = serde_json::from_str(&line)?;
        let socket = ready
            .pointer("/ready/socket")
            .and_then(Value::as_str)
            .ok_or("socket")?;
        let bearer = ready
            .pointer("/ready/bearer")
            .and_then(Value::as_str)
            .ok_or("bearer")?;
        let observer = ready
            .pointer("/ready/observer")
            .and_then(Value::as_str)
            .ok_or("observer")?
            .to_owned();
        let websocket = ready
            .pointer("/ready/websocket")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(Self {
            child,
            stdin,
            stdout,
            root,
            socket: socket.to_owned(),
            bearer: bearer.to_owned(),
            observer,
            websocket,
            ready,
        })
    }

    /// The parsed ready line.
    pub const fn ready(&self) -> &Value {
        &self.ready
    }

    /// Kills the server process this test spawned and reaps it; its state stays.
    ///
    /// # Errors
    ///
    /// Fails when the process cannot be signalled or reaped.
    pub fn crash(mut self) -> TestResult<PathBuf> {
        self.child.kill()?;
        self.child.wait()?;
        Ok(self.root)
    }

    /// Waits, with stdin still open, for the server to stop by itself and returns its
    /// report line.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    ///
    /// # Panics
    ///
    /// Panics when the server exits with a failure.
    pub fn stopped_by_itself(mut self) -> TestResult<Value> {
        let mut line = String::new();
        self.stdout.read_line(&mut line)?;
        let status = self.child.wait()?;
        assert_eq!(status.code(), Some(0), "serve exited {status:?}: {line}");
        drop(self.stdin.take());
        std::fs::remove_dir_all(&self.root)?;
        Ok(serde_json::from_str(&line)?)
    }

    /// Runs the binary with `--json` against this server's endpoint and bearer.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    pub fn cli(&self, args: &[&str]) -> TestResult<Output> {
        Ok(Command::new(BIN)
            .arg("--json")
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("OVIS_TERMINAL_SOCKET", &self.socket)
            .env("OVIS_TERMINAL_CREDENTIAL", &self.bearer)
            .stdin(Stdio::null())
            .output()?)
    }

    /// Runs the binary and parses its stdout, panicking with argv and both streams on a
    /// different exit code.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    ///
    /// # Panics
    ///
    /// Panics when the binary's behaviour differs from the expectation.
    pub fn ok(&self, args: &[&str]) -> TestResult<Value> {
        let output = self.cli(args)?;
        assert_eq!(output.status.code(), Some(0), "{args:?}: {output:?}");
        Ok(serde_json::from_slice(&output.stdout)?)
    }

    /// Runs the binary expecting `code` and returns the parsed stderr error record.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    ///
    /// # Panics
    ///
    /// Panics when the binary's behaviour differs from the expectation.
    pub fn refused(&self, args: &[&str], code: i32) -> TestResult<Value> {
        let output = self.cli(args)?;
        assert_eq!(output.status.code(), Some(code), "{args:?}: {output:?}");
        let record: Value = serde_json::from_slice(&output.stderr)?;
        Ok(record.get("error").cloned().ok_or("error record")?)
    }

    /// Launches `/bin/sh` named `name` with an explicit environment.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    pub fn run_shell(&self, name: &str) -> TestResult<Value> {
        let params = json!({
            "op_id": op_id()?, "program": {"argv": ["/bin/sh"]},
            "geometry": {"cols": 80, "rows": 24}, "cwd": "/tmp",
            "env": {"PATH": "/usr/bin:/bin", "PS1": "$ ", "TERM": "xterm"},
            "name": name, "labels": null, "parent": null, "child_policy": null,
            "keep": null, "first_message": null, "machine": null,
        });
        self.ok(&["call", "terminal.run", "--params", &params.to_string()])
    }

    /// The raw reply to a hello presenting `bearer` on the local socket.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    pub fn hello_reply(&self, bearer: &str) -> TestResult<Value> {
        let mut stream = UnixStream::connect(&self.socket)?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        let hello = json!({"jsonrpc": "2.0", "id": "h", "method": "hello", "params": {
            "generation": "1", "client": {"name": "t31", "version": "0"}, "credential": bearer,
        }});
        writeln!(stream, "{hello}")?;
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line)?;
        Ok(serde_json::from_str(&line)?)
    }

    /// The method list hello advertises to the owner.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    pub fn advertised(&self) -> TestResult<BTreeSet<String>> {
        let reply = self.hello_reply(&self.bearer)?;
        let methods = reply
            .pointer("/result/methods")
            .and_then(Value::as_array)
            .ok_or_else(|| reply.to_string())?;
        Ok(methods
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect())
    }

    /// Closes stdin and returns the serve report after a clean exit.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    ///
    /// # Panics
    ///
    /// Panics when the binary's behaviour differs from the expectation.
    pub fn stop(self) -> TestResult<Value> {
        let (report, root) = self.stop_keeping_state()?;
        std::fs::remove_dir_all(root)?;
        Ok(report)
    }

    /// Closes stdin, as [`Served::stop`], and keeps the state for a later boot.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    ///
    /// # Panics
    ///
    /// Panics when the server exits with a failure.
    pub fn stop_keeping_state(mut self) -> TestResult<(Value, PathBuf)> {
        drop(self.stdin.take());
        let mut line = String::new();
        self.stdout.read_line(&mut line)?;
        let status = self.child.wait()?;
        assert_eq!(status.code(), Some(0), "serve exited {status:?}");
        Ok((serde_json::from_str(&line)?, self.root.clone()))
    }
}
