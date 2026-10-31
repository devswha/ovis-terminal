//! Starting one declared program and supervising it to a proven outcome.

use std::ffi::OsString;
use std::io::Read as _;
use std::path::PathBuf;
use std::process::Stdio;

use ovis_terminal_protocol::schema::methods::plugin::PluginRunState;
use tokio::sync::{mpsc, watch};

use super::PluginError;
use super::runs::RunHandle;

/// Variables copied from the runtime; credentials and client leases never reach a plugin.
const INHERITED: [&str; 3] = ["PATH", "SystemRoot", "LANG"];

/// One program start: no shell, a cleared environment and no standard input.
#[derive(Debug, Clone)]
pub(super) struct Launch {
    pub(super) program: PathBuf,
    pub(super) args: Vec<String>,
    pub(super) cwd: PathBuf,
    pub(super) env: Vec<(String, OsString)>,
}

impl Launch {
    pub(super) fn inherited_env() -> Vec<(String, OsString)> {
        INHERITED
            .iter()
            .filter_map(|name| std::env::var_os(name).map(|value| ((*name).to_owned(), value)))
            .collect()
    }
}

fn pump(mut reader: std::io::PipeReader, chunks: &mpsc::Sender<std::io::Result<Vec<u8>>>) {
    let mut buffer = vec![0_u8; 8 << 10];
    loop {
        let sent = match reader.read(&mut buffer) {
            Ok(0) => return,
            Ok(read) => chunks.blocking_send(Ok(buffer.get(..read).unwrap_or_default().to_vec())),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => chunks.blocking_send(Err(error)),
        };
        if sent.is_err() {
            return;
        }
    }
}

fn outcome(status: std::process::ExitStatus, killed: bool) -> (PluginRunState, Option<i32>) {
    match status.code() {
        _ if killed => (PluginRunState::Cancelled, None),
        Some(0) => (PluginRunState::Succeeded, Some(0)),
        code => (PluginRunState::Failed, code),
    }
}

/// Runs `launch` until it exits and its output reaches end of file, or until cancellation
/// kills it. The run settles from the exit status, never from what the program printed.
pub(super) async fn supervise(
    launch: Launch,
    run: &RunHandle,
    mut cancel: watch::Receiver<bool>,
) -> Result<(), PluginError> {
    let (reader, writer) = std::io::pipe().map_err(PluginError::SpawnFailed)?;
    let stderr = writer.try_clone().map_err(PluginError::SpawnFailed)?;
    let mut command = tokio::process::Command::new(&launch.program);
    command
        .args(&launch.args)
        .current_dir(&launch.cwd)
        .env_clear()
        .envs(launch.env)
        .stdin(Stdio::null())
        .stdout(writer)
        .stderr(stderr)
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(PluginError::SpawnFailed)?;
    // The command keeps the write ends; drop them so end of file means the program closed.
    drop(command);
    let (sender, mut chunks) = mpsc::channel(16);
    std::thread::Builder::new()
        .name("plugin-output".to_owned())
        .spawn(move || pump(reader, &sender))
        .map_err(|source| PluginError::Io {
            operation: "starting the output reader",
            source,
        })?;
    let mut status = None;
    let (mut eof, mut cancelled, mut killed) = (false, false, false);
    while status.is_none() || !(eof || cancelled) {
        tokio::select! {
            chunk = chunks.recv(), if !eof => match chunk {
                Some(Ok(bytes)) => run.append(&bytes)?,
                Some(Err(source)) => return Err(PluginError::Io { operation: "reading plugin output", source }),
                None => eof = true,
            },
            exit = child.wait(), if status.is_none() => {
                status = Some(exit.map_err(|source| PluginError::Io { operation: "waiting for the plugin", source })?);
            },
            _ = cancel.wait_for(|requested| *requested), if !cancelled => {
                cancelled = true;
                if status.is_none() {
                    // Only the direct child is signalled: its own pid, never a group or 0/1/-1.
                    child.start_kill().map_err(|source| PluginError::Io { operation: "cancelling the plugin", source })?;
                    killed = true;
                }
            },
        }
    }
    let (state, exit_code) = status.map_or((PluginRunState::Cancelled, None), |status| {
        outcome(status, killed)
    });
    run.settle(state, exit_code)
}
