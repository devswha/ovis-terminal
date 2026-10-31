//! A real holder PTY whose output, exit and input acknowledgements feed one wait hub.

use std::process::Command;
use std::time::Instant;

use ovis_terminal_core::input::receipts::Receipt;
use ovis_terminal_core::input::sequencer::{QueueLimits, Sequencer, View, holder_refusal};
use ovis_terminal_core::screen::{Screen, ScreenBounds, ScreenConfig};
use ovis_terminal_core::waits::Waits;
use ovis_terminal_holder::protocol::{
    CoreBuildId, HOLDER_PROTOCOL_GENERATION, Hello, HolderBuildId, HolderDescriptor,
    HolderInstanceId, Message, PreviousCoreState, Spawn, Spawned, TerminalId, TerminalIncarnation,
    TerminalTarget,
};
use ovis_terminal_holder::unix::{CoreLink, Holder, HolderConfig, HolderLimits};
use ovis_terminal_protocol::schema::methods::server::WriteSequence;
use ovis_terminal_vt::{Geometry, HistoryLimit, VtConfig};

use super::TestResult;
use super::fixture::bound;
use super::sequencer::{Fixture, process};

/// A program on a PTY whose holder messages are fed to `waits` as they arrive.
#[derive(Debug)]
pub struct Live {
    holder: Holder,
    link: CoreLink,
    spawned: Spawned,
    /// The program's wait hub.
    pub waits: Waits,
}

impl Drop for Live {
    fn drop(&mut self) {
        let shutdown = self.holder.shutdown();
        assert!(shutdown.is_ok(), "holder workers must join: {shutdown:?}");
    }
}

impl Live {
    /// Spawns `script` and feeds output until it printed `ARMED`.
    ///
    /// # Errors
    ///
    /// Fails when the holder refuses or the marker does not arrive in time.
    pub fn start(script: &str) -> TestResult<Self> {
        let holder = Holder::new(HolderConfig::new(
            HolderDescriptor::new(
                HolderBuildId::new("waits-test".into()),
                HolderInstanceId::new(uuid::Uuid::now_v7().to_string()),
            ),
            HolderLimits::DEFAULT,
        ));
        let link = holder.adopt(&Hello {
            generation: HOLDER_PROTOCOL_GENERATION,
            core_build: CoreBuildId::new("waits-core".into()),
            previous: PreviousCoreState::Crash,
        })?;
        let spawned = holder.spawn_wire(&Spawn {
            terminal_id: TerminalId::new("terminal:9".into()),
            terminal_incarnation: TerminalIncarnation::new(1),
            argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
            cwd: std::env::temp_dir(),
            env: [("PATH".into(), "/usr/bin:/bin".into())].into(),
            geometry: (80, 24),
        })?;
        let target = TerminalTarget {
            terminal_id: spawned.terminal_id.clone(),
            terminal_incarnation: spawned.terminal_incarnation,
            process_incarnation: Some(spawned.process_incarnation),
        };
        let config = ScreenConfig::new(
            VtConfig::new(Geometry::new(80, 24)?, HistoryLimit::new(100)),
            ScreenBounds::DEFAULT,
        );
        let waits = Waits::new(Screen::new(target, config)?);
        let live = Self {
            holder,
            link,
            spawned,
            waits,
        };
        let mut seen = Vec::new();
        live.pump_until(|message| {
            if let Some(Message::Output(output)) = message {
                seen.extend_from_slice(&output.bytes);
            }
            seen.windows(5).any(|window| window == b"ARMED")
        })?;
        Ok(live)
    }

    /// Receives one holder message and feeds it to the hub.
    fn pump(&self, deadline: Instant) -> TestResult<Message> {
        let left = deadline
            .checked_duration_since(Instant::now())
            .ok_or("holder bound elapsed")?;
        let message = self.link.recv_timeout(left)?;
        match &message {
            Message::Output(output) => {
                self.waits.ingest(output)?;
            }
            Message::Exit(exit) => {
                self.waits.finish(exit)?;
            }
            _ => {}
        }
        Ok(message)
    }

    /// Feeds holder messages until `done` holds, checked before the first and after each one.
    ///
    /// # Errors
    ///
    /// Fails when the bound elapses, the link closes or the hub refuses a message.
    pub fn pump_until(
        &self,
        mut done: impl FnMut(Option<&Message>) -> bool,
    ) -> TestResult<Option<Message>> {
        let deadline = bound()?;
        if done(None) {
            return Ok(None);
        }
        loop {
            let message = self.pump(deadline)?;
            if done(Some(&message)) {
                return Ok(Some(message));
            }
        }
    }

    /// Writes every admitted entry to the PTY, feeding all output on the way.
    ///
    /// # Errors
    ///
    /// Fails when the sequencer, the holder or the hub fails.
    pub fn drain(&self, sequencer: &mut Sequencer, view: &View<'_>) -> TestResult<Vec<Receipt>> {
        let mut changed = Vec::new();
        loop {
            let step = sequencer.next(view)?;
            changed.extend(step.fenced);
            let Some(input) = step.dispatch else {
                return Ok(changed);
            };
            let sequence = input.sequence;
            if let Err(error) = self.holder.input(input) {
                changed.extend(
                    sequencer.refused(WriteSequence::new(sequence), holder_refusal(&error)),
                );
                continue;
            }
            let acked = self.pump_until(|message| matches!(message, Some(Message::InputWritten(ack)) if ack.sequence == sequence))?;
            if let Some(Message::InputWritten(ack)) = acked {
                changed.extend(sequencer.acknowledge(&ack)?);
            }
        }
    }

    /// A sequencer live with the program's process incarnation.
    ///
    /// # Errors
    ///
    /// Fails when the fixture target is rejected.
    pub fn sequencer(&self, fixture: &Fixture) -> TestResult<Sequencer> {
        let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;
        let fenced = sequencer.stream_ready(
            process(self.spawned.process_incarnation.get())?,
            WriteSequence::new(0),
        );
        if !fenced.is_empty() {
            return Err("an empty sequencer fenced entries".into());
        }
        Ok(sequencer)
    }

    /// Independent oracle: whether the OS still lists the program's PID, via `ps`.
    ///
    /// # Errors
    ///
    /// Fails when `ps` cannot run.
    pub fn os_lists_process(&self) -> TestResult<bool> {
        let status = Command::new("ps")
            .args(["-p", &self.spawned.pid.to_string()])
            .output()?
            .status;
        Ok(status.success())
    }
}
