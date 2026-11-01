//! One child process run to completion under a hard deadline.
//!
//! Completion is the event both output pipes reach end of file; the deadline is
//! only a hang guard. On the deadline the runner kills the child it spawned (its
//! own handle, never a pid lookup) and reports `timed_out`.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use super::error::QaError;

/// Bound on collecting output after the exit or the kill.
const AFTER_KILL: Duration = Duration::from_secs(10);

/// What a bounded run captured.
#[derive(Debug)]
#[non_exhaustive]
pub struct Bounded {
    /// Exit code; absent when the child was killed or ended by a signal.
    pub exit_code: Option<i32>,
    /// The deadline ended the run.
    pub timed_out: bool,
    /// Captured standard output.
    pub stdout: Vec<u8>,
    /// Captured standard error.
    pub stderr: Vec<u8>,
}

type Pipe = (usize, Vec<u8>);

fn drain(reader: impl Read + Send + 'static, slot: usize, sender: mpsc::Sender<Pipe>) {
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut bytes = Vec::new();
        let note = match reader.read_to_end(&mut bytes) {
            Ok(_) => Vec::new(),
            Err(error) => format!("<read failed: {error}>").into_bytes(),
        };
        bytes.extend(note);
        drop(sender.send((slot, bytes)));
    });
}

fn collect(receiver: &Receiver<Pipe>, (start, bound): (Instant, Duration)) -> [Option<Vec<u8>>; 2] {
    let mut pipes = [None, None];
    while pipes.iter().any(Option::is_none) {
        let left = bound.saturating_sub(start.elapsed());
        let Ok((slot, bytes)) = receiver.recv_timeout(left) else {
            break;
        };
        if let Some(entry) = pipes.get_mut(slot) {
            *entry = Some(bytes);
        }
    }
    pipes
}

fn reap(child: &mut Child) -> Result<Option<i32>, QaError> {
    child
        .wait()
        .map(|status| status.code())
        .map_err(|source| QaError::Io {
            operation: "wait for the child",
            source,
        })
}

/// Runs `command` with piped output until both pipes close, or kills it at `deadline`.
///
/// # Errors
///
/// The child cannot be spawned, waited for or killed, or reports pid 0 or 1.
pub fn run(command: &mut Command, deadline: Duration) -> Result<Bounded, QaError> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| QaError::Io {
            operation: "spawn the child",
            source,
        })?;
    let pid = child.id();
    if pid <= 1 {
        return Err(QaError::UnsafePid { pid });
    }
    let (sender, receiver) = mpsc::channel();
    if let Some(out) = child.stdout.take() {
        drain(out, 0, sender.clone());
    }
    if let Some(err) = child.stderr.take() {
        drain(err, 1, sender);
    }
    let [stdout, stderr] = collect(&receiver, (Instant::now(), deadline));
    let timed_out = stdout.is_none() || stderr.is_none();
    let exit_code = if timed_out {
        child.kill().map_err(|source| QaError::Io {
            operation: "kill the child at its deadline",
            source,
        })?;
        reap(&mut child)?;
        None
    } else {
        reap(&mut child)?
    };
    let late = collect(&receiver, (Instant::now(), AFTER_KILL));
    let [late_out, late_err] = late;
    let unread = || b"<unread after the deadline>".to_vec();
    Ok(Bounded {
        exit_code,
        timed_out,
        stdout: stdout.or(late_out).unwrap_or_else(unread),
        stderr: stderr.or(late_err).unwrap_or_else(unread),
    })
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::Duration;

    use super::run;

    /// This test binary re-invoked as a portable child: it lists its tests and exits 0.
    fn listing() -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--list", "--format", "terse"]);
        command
    }

    #[test]
    fn a_child_that_exits_reports_its_code_and_output() {
        let done = run(&mut listing(), Duration::from_secs(120)).unwrap();
        assert_eq!(done.exit_code, Some(0), "{done:?}");
        assert!(!done.timed_out);
        let listed = String::from_utf8(done.stdout).unwrap();
        assert!(
            listed.contains("a_child_that_exits_reports_its_code_and_output: test"),
            "{listed}"
        );
    }

    #[test]
    fn a_child_still_running_at_the_deadline_is_killed_and_reported() {
        let mut stalled = Command::new(std::env::current_exe().unwrap());
        stalled.env("OVIS_QA_STALL", "1").args([
            "--exact",
            "platform_qa::bounded::tests::stall_child",
            "--include-ignored",
        ]);
        let done = run(&mut stalled, Duration::ZERO).unwrap();
        assert!(done.timed_out, "{done:?}");
        assert_eq!(done.exit_code, None);
    }

    /// The stalled child of the deadline test: it parks forever.
    #[test]
    #[ignore = "child of a_child_still_running_at_the_deadline_is_killed_and_reported"]
    fn stall_child() {
        if std::env::var_os("OVIS_QA_STALL").is_some() {
            loop {
                std::thread::park();
            }
        }
    }
}
