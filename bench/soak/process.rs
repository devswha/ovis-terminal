//! Child processes the harness started: the first stdout line read within a bound, the
//! rest copied to a log, and exit awaited within a bound. Only PIDs captured at spawn
//! (always greater than 1) are ever signalled.

use std::io::{BufRead as _, BufReader, Read as _};
use std::path::Path;
use std::process::{Child, ChildStdout, ExitStatus};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use ovis_terminal_probes::platform::{Pid, force_kill};

use super::error::SoakError;

/// A started child, its PID and the thread copying its stdout after the first line.
#[derive(Debug)]
pub(super) struct Started {
    pub(super) child: Child,
    pub(super) pid: u32,
    pub(super) first_line: String,
    copier: JoinHandle<std::io::Result<()>>,
}

fn copy_rest(mut reader: BufReader<ChildStdout>, log: &Path) -> std::io::Result<()> {
    let mut rest = Vec::new();
    reader.read_to_end(&mut rest)?;
    if rest.is_empty() {
        return Ok(());
    }
    std::fs::write(log, &rest)
}

fn join(copier: JoinHandle<std::io::Result<()>>) -> Result<(), SoakError> {
    copier
        .join()
        .map_err(|_| SoakError::Protocol {
            expected: "a stdout reader that ends",
        })?
        .map_err(SoakError::io("copy a child's stdout"))
}

/// Takes `child`'s stdout, waits at most `bound` for its first line, then copies the
/// rest to `log`. A child that stays silent is killed and reaped.
pub(super) fn first_line(
    mut child: Child,
    (bound, waiting_for): (Duration, &'static str),
    log: &Path,
) -> Result<Started, SoakError> {
    let pid = child.id();
    if pid <= 1 {
        return Err(SoakError::Protocol {
            expected: "a started child with a PID above 1",
        });
    }
    let stdout = child.stdout.take().ok_or(SoakError::Protocol {
        expected: "a piped stdout",
    })?;
    let (sender, receiver) = mpsc::channel();
    let log = log.to_path_buf();
    let copier = std::thread::Builder::new()
        .name("soak-child-stdout".to_owned())
        .spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            let read = reader.read_line(&mut line).map(|_| line);
            if sender.send(read).is_err() {
                return Ok(());
            }
            copy_rest(reader, &log)
        })
        .map_err(SoakError::io("start a stdout reader"))?;
    match receiver.recv_timeout(bound) {
        Ok(Ok(line)) if !line.is_empty() => Ok(Started {
            child,
            pid,
            first_line: line,
            copier,
        }),
        Ok(_) => {
            kill(&mut child, pid)?;
            join(copier)?;
            Err(SoakError::ChildExited { waiting_for })
        }
        Err(_) => {
            kill(&mut child, pid)?;
            join(copier)?;
            Err(SoakError::Timeout {
                waiting_for,
                bound_ms: u64::try_from(bound.as_millis()).unwrap_or(u64::MAX),
            })
        }
    }
}

/// SIGKILL to `pid` (captured at spawn, above 1), then reaps it.
pub(super) fn kill(child: &mut Child, pid: u32) -> Result<ExitStatus, SoakError> {
    if pid <= 1 || child.id() != pid {
        return Err(SoakError::Protocol {
            expected: "the PID captured at spawn",
        });
    }
    if child
        .try_wait()
        .map_err(SoakError::io("check a child"))?
        .is_none()
    {
        force_kill(Pid::new(pid))?;
    }
    child.wait().map_err(SoakError::io("reap a killed child"))
}

impl Started {
    /// Waits at most `bound` for the child to exit on its own; past the bound it is
    /// killed and the timeout is reported.
    pub(super) fn wait(
        self,
        (bound, waiting_for): (Duration, &'static str),
    ) -> Result<ExitStatus, SoakError> {
        let Self {
            mut child,
            pid,
            copier,
            ..
        } = self;
        let (sender, receiver) = mpsc::channel();
        let waiter = std::thread::Builder::new()
            .name("soak-child-wait".to_owned())
            .spawn(move || {
                let status = child.wait();
                let sent = sender.send(());
                (child, status, sent.is_ok())
            })
            .map_err(SoakError::io("start a child waiter"))?;
        let timed_out = receiver.recv_timeout(bound).is_err();
        if timed_out {
            force_kill(Pid::new(pid))?;
        }
        let (_, status, _) = waiter.join().map_err(|_| SoakError::Protocol {
            expected: "a child waiter that ends",
        })?;
        join(copier)?;
        let status = status.map_err(SoakError::io("wait for a child"))?;
        if timed_out {
            return Err(SoakError::Timeout {
                waiting_for,
                bound_ms: u64::try_from(bound.as_millis()).unwrap_or(u64::MAX),
            });
        }
        Ok(status)
    }

    /// SIGKILL, reap, and join the stdout copier.
    pub(super) fn kill(self) -> Result<ExitStatus, SoakError> {
        let Self {
            mut child,
            pid,
            copier,
            ..
        } = self;
        let status = kill(&mut child, pid)?;
        join(copier)?;
        Ok(status)
    }
}
