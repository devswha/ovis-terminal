//! Bounded service-manager calls: a hung manager never hangs the caller.
//!
//! On expiry only the spawned child is killed, through its own handle; no process group
//! or guessed PID is ever signalled.

use std::ffi::OsStr;
use std::process::{Child, Command, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::ServiceError;

const POLL: Duration = Duration::from_millis(5);

/// The captured result of one manager call that exited.
#[derive(Debug)]
pub(super) struct Called {
    pub(super) success: bool,
    pub(super) code: Option<i32>,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

fn drain(pipe: Option<impl std::io::Read + Send + 'static>) -> JoinHandle<String> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let failure = pipe.and_then(|mut pipe| pipe.read_to_end(&mut bytes).err());
        let text = String::from_utf8_lossy(&bytes);
        failure.map_or_else(
            || text.to_string(),
            |error| format!("{text}<read failed: {error}>"),
        )
    })
}

fn bounded_wait(
    child: &mut Child,
    bound: Duration,
) -> Result<Option<std::process::ExitStatus>, std::io::Error> {
    let deadline = Instant::now().checked_add(bound);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if deadline.is_none_or(|deadline| Instant::now() >= deadline) {
            return Ok(None);
        }
        std::thread::sleep(POLL);
    }
}

/// Runs `program args` with no stdin and waits at most `bound`.
///
/// # Errors
///
/// A program that cannot start is [`ServiceError::Io`]; expiry is
/// [`ServiceError::ManagerTimeout`] after the child is killed and reaped.
pub(super) fn call(
    program: &'static str,
    args: &[&OsStr],
    bound: Duration,
) -> Result<Called, ServiceError> {
    let io = |operation| move |source| ServiceError::Io { operation, source };
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(io("start the service manager"))?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let waited = bounded_wait(&mut child, bound);
    let status = if let Ok(Some(status)) = waited {
        Some(status)
    } else {
        child.kill().map_err(io("end the expired manager call"))?;
        child.wait().map_err(io("reap the expired manager call"))?;
        None
    };
    let joined = |reader: JoinHandle<String>| {
        reader
            .join()
            .unwrap_or_else(|_| "<reader panicked>".to_owned())
    };
    let (stdout, stderr) = (joined(stdout), joined(stderr));
    let Some(status) = status else {
        return Err(ServiceError::ManagerTimeout {
            program,
            bound_ms: u64::try_from(bound.as_millis()).unwrap_or(u64::MAX),
        });
    };
    Ok(Called {
        success: status.success(),
        code: status.code(),
        stdout,
        stderr: stderr.trim().to_owned(),
    })
}

/// [`call`] that turns an unsuccessful exit into [`ServiceError::ManagerFailed`].
pub(super) fn checked(
    program: &'static str,
    args: &[&OsStr],
    bound: Duration,
) -> Result<Called, ServiceError> {
    let called = call(program, args, bound)?;
    if called.success {
        return Ok(called);
    }
    Err(ServiceError::ManagerFailed {
        program,
        code: called.code,
        stderr: called.stderr,
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::time::{Duration, Instant};

    use super::{call, checked};

    #[test]
    fn a_hung_manager_is_killed_at_the_bound_and_reported_as_unknown_outcome() {
        let started = Instant::now();
        let hung = call(
            "/bin/sleep",
            &[OsStr::new("30")],
            Duration::from_millis(200),
        );
        assert_eq!(hung.map(drop).map_err(|e| e.kind()), Err("manager_timeout"));
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the bound was not enforced"
        );
    }

    #[test]
    fn exit_status_and_diagnostics_are_reported_not_guessed() {
        let args = [
            OsStr::new("-c"),
            OsStr::new("echo out; echo err >&2; exit 3"),
        ];
        let called = call("/bin/sh", &args, Duration::from_secs(10)).unwrap();
        assert_eq!(
            (
                called.success,
                called.code,
                called.stdout.as_str(),
                called.stderr.as_str()
            ),
            (false, Some(3), "out\n", "err")
        );
        let failed = checked("/bin/sh", &args, Duration::from_secs(10))
            .map(drop)
            .map_err(|e| e.kind());
        assert_eq!(failed, Err("manager_failed"));
    }
}
