//! A real holder running one program on a PTY, its output fed into the terminal's screen.

use std::time::{Duration, Instant};

use ovis_terminal_core::screen::Screen;
use ovis_terminal_holder::protocol::{
    CoreBuildId, HOLDER_PROTOCOL_GENERATION, Hello, HolderBuildId, HolderDescriptor,
    HolderInstanceId, InputWritten, Message, PreviousCoreState, Spawn, Spawned, TerminalId,
    TerminalIncarnation, TerminalTarget,
};
use ovis_terminal_holder::unix::{CoreLink, Holder, HolderConfig, HolderLimits};

use super::TestResult;
use super::rig::screen;

/// A holder whose processes are killed and workers joined when the test ends.
#[derive(Debug)]
pub struct Pty {
    /// The holder.
    pub holder: Holder,
    link: CoreLink,
    spawned: Spawned,
    /// The terminal's screen, fed every output chunk in order.
    pub screen: Screen,
    /// Raw program output from offset zero.
    pub output: Vec<u8>,
    /// OSC records the program emitted; a readiness report would land here.
    pub osc: usize,
    /// Write acknowledgements not yet consumed.
    pub acks: Vec<InputWritten>,
}

impl Drop for Pty {
    fn drop(&mut self) {
        let shutdown = self.holder.shutdown();
        assert!(shutdown.is_ok(), "holder workers must join: {shutdown:?}");
    }
}

impl Pty {
    /// Spawns `argv` as terminal 9 at `grid` with a plain environment.
    ///
    /// # Errors
    ///
    /// Fails when the holder refuses the program.
    pub fn start(argv: &[&str], grid: (u16, u16)) -> TestResult<Self> {
        let holder = Holder::new(HolderConfig::new(
            HolderDescriptor::new(
                HolderBuildId::new("geometry-test".into()),
                HolderInstanceId::new(uuid::Uuid::now_v7().to_string()),
            ),
            HolderLimits::DEFAULT,
        ));
        let link = holder.adopt(&Hello {
            generation: HOLDER_PROTOCOL_GENERATION,
            core_build: CoreBuildId::new("geometry-core".into()),
            previous: PreviousCoreState::Crash,
        })?;
        let spawned = holder.spawn_wire(&Spawn {
            terminal_id: TerminalId::new("terminal:9".into()),
            terminal_incarnation: TerminalIncarnation::new(1),
            argv: argv.iter().map(|arg| (*arg).to_owned()).collect(),
            cwd: std::env::temp_dir(),
            env: [
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("TERM".into(), "xterm-256color".into()),
                ("PS1".into(), "PROMPT$ ".into()),
            ]
            .into(),
            geometry: grid,
        })?;
        let target = TerminalTarget {
            terminal_id: spawned.terminal_id.clone(),
            terminal_incarnation: spawned.terminal_incarnation,
            process_incarnation: Some(spawned.process_incarnation),
        };
        let screen = screen(target, grid.0, grid.1)?;
        Ok(Self {
            holder,
            link,
            spawned,
            screen,
            output: Vec::new(),
            osc: 0,
            acks: Vec::new(),
        })
    }

    /// The program's process incarnation.
    pub const fn process(&self) -> u64 {
        self.spawned.process_incarnation.get()
    }

    /// Receives holder messages into the screen until `done` holds or `bound` elapses.
    ///
    /// # Errors
    ///
    /// Fails when the bound elapses, the link closes or the screen rejects output.
    pub fn until(&mut self, bound: Duration, mut done: impl FnMut(&Self) -> bool) -> TestResult {
        let deadline = Instant::now().checked_add(bound).ok_or("clock overflow")?;
        while !done(self) {
            let left = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| {
                    format!(
                        "holder bound elapsed; output so far: {:?}",
                        String::from_utf8_lossy(&self.output)
                    )
                })?;
            match self.link.recv_timeout(left)? {
                Message::Output(output) => {
                    let ingested = self.screen.ingest(&output)?;
                    self.osc = self.osc.saturating_add(ingested.osc.len());
                    self.output.extend_from_slice(&output.bytes);
                }
                Message::InputWritten(ack) => self.acks.push(ack),
                _ => {}
            }
        }
        Ok(())
    }

    /// Whether the raw output contains `needle`.
    pub fn printed(&self, needle: &[u8]) -> bool {
        self.output
            .windows(needle.len())
            .any(|window| window == needle)
    }

    /// Raw output offset of the first `needle`.
    pub fn offset_of(&self, needle: &[u8]) -> Option<usize> {
        self.output
            .windows(needle.len())
            .position(|window| window == needle)
    }
}
