//! A running `serve` with one shell, and an outer PTY driving one attach client.

use std::fs::File;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::process::CommandExt as _;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rustix::termios::{self, Winsize};
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

/// An outer PTY whose slave is the controlling terminal of one attach client.
#[derive(Debug)]
pub struct Outer {
    /// The PTY master: what the test types and reads.
    pub master: File,
    slave: File,
    output: mpsc::Receiver<Vec<u8>>,
    seen: Vec<u8>,
    original: String,
    /// The attach client process.
    pub client: Child,
}

fn settings(file: &File) -> TestResult<String> {
    Ok(format!("{:?}", termios::tcgetattr(file)?))
}

impl Outer {
    /// Starts the binary with `args` on a fresh 100x30 outer PTY as its controlling terminal.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    pub fn attach(served: &Served, args: &[&str]) -> TestResult<Self> {
        let master = File::from(rustix::pty::openpt(
            rustix::pty::OpenptFlags::RDWR | rustix::pty::OpenptFlags::NOCTTY,
        )?);
        rustix::pty::grantpt(&master)?;
        rustix::pty::unlockpt(&master)?;
        let name = rustix::pty::ptsname(&master, Vec::new())?;
        let slave = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(name.to_str()?)?;
        termios::tcsetwinsize(
            &master,
            Winsize {
                ws_row: 30,
                ws_col: 100,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )?;
        let original = settings(&slave)?;
        let mut command = served.command(args);
        command
            .stdin(slave.try_clone()?)
            .stdout(slave.try_clone()?)
            .stderr(Stdio::piped());
        // SAFETY: between fork and exec the closure only calls async-signal-safe setsid and
        // the TIOCSCTTY ioctl on its own stdin, making the PTY slave its controlling terminal.
        unsafe {
            command.pre_exec(|| {
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(rustix::stdio::stdin())?;
                Ok(())
            });
        }
        let client = command.spawn()?;
        let (sender, output) = mpsc::channel();
        let mut reader = master.try_clone()?;
        std::thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0
                    || sender
                        .send(buffer.get(..read).map(<[u8]>::to_vec).unwrap_or_default())
                        .is_err()
                {
                    break;
                }
            }
        });
        Ok(Self {
            master,
            slave,
            output,
            seen: Vec::new(),
            original,
            client,
        })
    }

    /// Types `bytes` on the outer terminal.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    pub fn type_bytes(&mut self, bytes: &[u8]) -> TestResult {
        Ok(self.master.write_all(bytes)?)
    }

    /// Waits, bounded, until `needle` appears in what the client drew on the outer PTY.
    pub fn shows(&mut self, needle: &str, within: Duration) -> bool {
        let Some(deadline) = Instant::now().checked_add(within) else {
            return false;
        };
        loop {
            if self
                .seen
                .windows(needle.len())
                .any(|window| window == needle.as_bytes())
            {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self.output.recv_timeout(left) {
                Ok(chunk) => self.seen.extend(chunk),
                Err(_) => return false,
            }
        }
    }

    /// Waits for the client to exit; returns its code and parsed stderr record.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    ///
    /// # Panics
    ///
    /// Panics when the binary's behaviour differs from the expectation.
    pub fn exited(self) -> TestResult<(Option<i32>, Value, bool)> {
        let Self {
            slave,
            original,
            mut client,
            ..
        } = self;
        let pid = client.id();
        let (sender, exited) = mpsc::channel();
        std::thread::spawn(move || {
            let mut stderr = String::new();
            if let Some(mut pipe) = client.stderr.take() {
                pipe.read_to_string(&mut stderr).ok();
            }
            sender
                .send((client.wait().map(|status| status.code()), stderr))
                .ok();
        });
        let Ok((status, stderr)) = exited.recv_timeout(WAIT) else {
            let pid = rustix::process::Pid::from_raw(i32::try_from(pid)?).ok_or("pid")?;
            assert!(pid.as_raw_nonzero().get() > 1);
            rustix::process::kill_process(pid, rustix::process::Signal::KILL)?;
            return Err("the attach client did not exit".into());
        };
        let record =
            serde_json::from_str(stderr.lines().last().unwrap_or("null")).unwrap_or(Value::Null);
        let restored = settings(&slave)? == original;
        Ok((status?, record, restored))
    }
}
