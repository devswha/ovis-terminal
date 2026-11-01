//! One real pseudo-terminal per terminal: `terminal50 emit <rate>` in a private
//! environment, an owned pump thread into the actor and an owned stdin forwarder.
//!
//! The emitter never reads stdin, so admitted input fills the terminal's input buffer:
//! the stdin non-reader case. It runs until killed; only its captured pid is signalled.

use std::io::Write as _;
use std::path::Path;
use std::thread::{self, JoinHandle};

use ovis_terminal_probes::platform::Pid;
use ovis_terminal_server::streams::scheduler::{PumpReport, PumpThread};
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};
use tokio::io::AsyncReadExt as _;

use super::error::Terminal50Error;
use super::fleet::Wiring;

const SIZE: PtySize = PtySize {
    rows: 40,
    cols: 120,
    pixel_width: 0,
    pixel_height: 0,
};

#[derive(Debug, Clone, Copy)]
pub(super) struct Emit<'a> {
    pub(super) exe: &'a Path,
    pub(super) root: &'a Path,
    pub(super) bytes_per_second: usize,
}

pub(super) struct EmitTerminal {
    child: Box<dyn Child + Send + Sync>,
    pump: PumpThread,
    forward: JoinHandle<u64>,
    pid: Pid,
}

impl std::fmt::Debug for EmitTerminal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EmitTerminal")
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

impl EmitTerminal {
    pub(super) fn spawn(emit: Emit<'_>, wiring: Wiring) -> Result<Self, Terminal50Error> {
        let pair = native_pty_system()
            .openpty(SIZE)
            .map_err(Terminal50Error::pty("open a pseudo-terminal"))?;
        let script = format!(
            "stty -echo; exec '{}' emit --bytes-per-second {}",
            emit.exe.display(),
            emit.bytes_per_second
        );
        let mut command = CommandBuilder::new("sh");
        command.args(["-c", &script]);
        command.env_clear();
        command.env("PATH", "/usr/bin:/bin");
        command.env("HOME", emit.root);
        command.env("TMPDIR", emit.root);
        command.env("XDG_RUNTIME_DIR", emit.root);
        command.cwd(emit.root);
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(Terminal50Error::pty("spawn the emitter"))?;
        drop(pair.slave);
        let pid = child
            .process_id()
            .filter(|pid| *pid > 1)
            .map(Pid::new)
            .ok_or(Terminal50Error::Pty {
                stage: "capture the emitter pid",
                source: "the backend reported no usable pid".into(),
            })?;
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(Terminal50Error::pty("clone the output reader"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(Terminal50Error::pty("take the input writer"))?;
        let pump = PumpThread::spawn("terminal50-pump", reader, wiring.feed)?;
        let Wiring { stdin, runtime, .. } = wiring;
        let forward = thread::Builder::new()
            .name("terminal50-stdin".to_owned())
            .spawn(move || {
                let mut stdin = stdin;
                let mut writer = writer;
                let mut buffer = vec![0_u8; 16 * 1024];
                let mut total = 0_u64;
                while let Ok(read) = runtime.block_on(stdin.read(&mut buffer)) {
                    let Some(bytes) = buffer.get(..read).filter(|bytes| !bytes.is_empty()) else {
                        break;
                    };
                    if writer.write_all(bytes).is_err() {
                        break;
                    }
                    total = total.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
                }
                total
            })?;
        Ok(Self {
            child,
            pump,
            forward,
            pid,
        })
    }

    pub(super) const fn pid(&self) -> Pid {
        self.pid
    }

    /// Kills the emitter it spawned, reaps it and joins its pump.
    pub(super) fn finish(mut self) -> Result<(PumpReport, JoinHandle<u64>), Terminal50Error> {
        self.child
            .kill()
            .map_err(Terminal50Error::pty("kill the emitter"))?;
        self.child.wait()?;
        let pumped = self.pump.join()?;
        Ok((pumped, self.forward))
    }
}
