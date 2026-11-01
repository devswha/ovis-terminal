//! Opening the Windows named pipe as the platform contract requires of a client.
//!
//! The server keeps one waiting instance and creates the next as soon as a client
//! takes it, so a second client can arrive in between and be refused with
//! `ERROR_PIPE_BUSY`. The client then waits in `WaitNamedPipeW` for a free instance,
//! bounded by what is left of the connect bound, and opens again; after the bound it
//! reports `pipe_busy_timeout` instead of a generic transport failure.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;
use windows_sys::Win32::System::Pipes::WaitNamedPipeW;

use super::refused;
use crate::invocation::error::CliError;

/// Bound on waiting for a free pipe instance, inside the 10 s exchange deadline.
pub(super) const BUSY_BOUND: Duration = Duration::from_secs(5);

fn is_busy(error: &std::io::Error) -> bool {
    error
        .raw_os_error()
        .and_then(|code| u32::try_from(code).ok())
        == Some(ERROR_PIPE_BUSY)
}

/// Blocks until `name` has a free instance or `wait` passes; either way the caller
/// opens again, and a still-busy pipe is decided by the caller's bound.
async fn wait_for_instance(name: &str, wait: Duration) -> Result<(), CliError> {
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let timeout_ms = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX).max(1);
    tokio::task::spawn_blocking(move || {
        // SAFETY: `wide` is NUL-terminated and owned by this closure for the whole call.
        unsafe { WaitNamedPipeW(wide.as_ptr(), timeout_ms) }
    })
    .await
    .map(drop)
    .map_err(|join| CliError::Io {
        operation: "wait for a free pipe instance",
        source: std::io::Error::other(join),
    })
}

/// Opens `name`, waiting out `ERROR_PIPE_BUSY` for at most `bound`.
pub(super) async fn open(name: &str, bound: Duration) -> Result<NamedPipeClient, CliError> {
    let started = Instant::now();
    loop {
        let error = match ClientOptions::new().open(name) {
            Ok(pipe) => return Ok(pipe),
            Err(error) if is_busy(&error) => error,
            Err(error) => return Err(refused(PathBuf::from(name), error)),
        };
        let left = bound.saturating_sub(started.elapsed());
        if left.is_zero() {
            return Err(CliError::PipeBusy {
                pipe: name.to_owned(),
                waited_ms: u64::try_from(bound.as_millis()).unwrap_or(u64::MAX),
                source: error,
            });
        }
        wait_for_instance(name, left).await?;
    }
}

#[cfg(test)]
#[path = "pipe_tests.rs"]
mod tests;
