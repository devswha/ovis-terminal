//! One real pseudo-terminal per load terminal: a paced emitter whose output an owned pump
//! thread hands to the actor, and whose stdin is fed by an owned forwarding thread.
//!
//! The emitter turns echo off and never reads stdin, so admitted input fills the terminal's
//! input buffer and then blocks the forwarder: the stdin non-reader case.

use std::io::Write as _;
use std::path::Path;
use std::thread::{self, JoinHandle};

use ovis_terminal_probes::platform::Pid;
use ovis_terminal_server::streams::scheduler::{OutputFeed, PumpReport, PumpThread};
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};
use tokio::io::{AsyncReadExt as _, DuplexStream};
use tokio::runtime::Handle;

use super::error::TuningError;

const SIZE: PtySize = PtySize {
    rows: 40,
    cols: 120,
    pixel_width: 0,
    pixel_height: 0,
};

/// What the emitter writes: `bytes_per_second` for `seconds`.
#[derive(Debug, Clone, Copy)]
pub(super) struct Emit {
    pub(super) bytes_per_second: usize,
    pub(super) seconds: u64,
}

/// The actor-side ends one terminal is wired to.
#[derive(Debug)]
pub(super) struct Wiring {
    pub(super) feed: OutputFeed,
    pub(super) stdin: DuplexStream,
    pub(super) runtime: Handle,
}

pub(super) struct EmitTerminal {
    child: Box<dyn Child + Send + Sync>,
    pump: PumpThread,
    forward: JoinHandle<u64>,
    pid: Pid,
}

impl EmitTerminal {
    pub(super) fn spawn(exe: &Path, emit: Emit, wiring: Wiring) -> Result<Self, TuningError> {
        let pair = native_pty_system()
            .openpty(SIZE)
            .map_err(TuningError::pty("open a pseudo-terminal"))?;
        let script = format!(
            "stty -echo; exec '{}' emit {} {}",
            exe.display(),
            emit.bytes_per_second,
            emit.seconds
        );
        let mut command = CommandBuilder::new("sh");
        command.args(["-c", &script]);
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(TuningError::pty("spawn the emitter"))?;
        drop(pair.slave);
        let pid = Pid::new(child.process_id().unwrap_or_default());
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(TuningError::pty("clone the output reader"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(TuningError::pty("take the input writer"))?;
        let pump = PumpThread::spawn("tuning-pump", reader, wiring.feed)?;
        let Wiring { stdin, runtime, .. } = wiring;
        let forward = thread::Builder::new()
            .name("tuning-stdin".to_owned())
            .spawn(move || forward(stdin, writer, &runtime))?;
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

    /// Waits for the emitter and joins its pump; join the forwarder once the actor stopped.
    pub(super) fn finish(mut self) -> Result<(PumpReport, Forwarder), TuningError> {
        self.child.wait()?;
        let pumped = self.pump.join()?;
        Ok((pumped, Forwarder(self.forward)))
    }
}

/// The stdin forwarding thread; it ends when the actor's input stream closes.
pub(super) struct Forwarder(JoinHandle<u64>);

impl Forwarder {
    /// Bytes forwarded into the terminal, once the thread ended.
    pub(super) fn join(self) -> u64 {
        self.0.join().unwrap_or_default()
    }
}

/// Copies the actor's input stream into the terminal until either side ends.
fn forward(
    mut stdin: DuplexStream,
    mut writer: Box<dyn std::io::Write + Send>,
    runtime: &Handle,
) -> u64 {
    let mut buffer = vec![0_u8; 16 * 1024];
    let mut total = 0_u64;
    loop {
        let read = match runtime.block_on(stdin.read(&mut buffer)) {
            Ok(0) | Err(_) => return total,
            Ok(read) => read,
        };
        let bytes = buffer.get(..read).unwrap_or_default();
        if writer.write_all(bytes).is_err() {
            return total;
        }
        total = total.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
    }
}
