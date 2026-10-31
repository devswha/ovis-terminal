//! A real holder adopted by the test and one program on a PTY.

use std::time::{Duration, Instant};

use ovis_terminal_core::input::receipts::Receipt;
use ovis_terminal_core::input::sequencer::{QueueLimits, Sequencer, View, holder_refusal};
use ovis_terminal_holder::protocol::{
    CoreBuildId, HOLDER_PROTOCOL_GENERATION, Hello, HolderBuildId, HolderDescriptor,
    HolderInstanceId, Message, PreviousCoreState, Spawn, Spawned, TerminalId, TerminalIncarnation,
    TerminalTarget,
};
use ovis_terminal_holder::unix::{CoreLink, Holder, HolderConfig, HolderLimits};
use ovis_terminal_protocol::schema::methods::server::WriteSequence;

use super::TestResult;
use super::fixture::{Fixture, process};

const BOUND: Duration = Duration::from_secs(20);

/// A holder whose processes are killed and workers joined when the test ends.
#[derive(Debug)]
pub struct Pty {
    /// The holder.
    pub holder: Holder,
    link: CoreLink,
    spawned: Spawned,
    /// Program output since `ARMED`.
    pub output: Vec<u8>,
}

impl Drop for Pty {
    fn drop(&mut self) {
        let shutdown = self.holder.shutdown();
        assert!(shutdown.is_ok(), "holder workers must join: {shutdown:?}");
    }
}

impl Pty {
    /// Spawns `script` as terminal 9 and waits until it printed `ARMED`.
    ///
    /// # Errors
    ///
    /// Fails when the holder refuses or the marker does not arrive in time.
    pub fn start(script: &str) -> TestResult<Self> {
        let holder = Holder::new(HolderConfig::new(
            HolderDescriptor::new(
                HolderBuildId::new("input-test".into()),
                HolderInstanceId::new(uuid::Uuid::now_v7().to_string()),
            ),
            HolderLimits::DEFAULT,
        ));
        let link = holder.adopt(&Hello {
            generation: HOLDER_PROTOCOL_GENERATION,
            core_build: CoreBuildId::new("input-core".into()),
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
        let mut pty = Self {
            holder,
            link,
            spawned,
            output: Vec::new(),
        };
        pty.until(|pty, _| pty.output.windows(5).any(|window| window == b"ARMED"))?;
        pty.output.clear();
        Ok(pty)
    }

    /// The running program as a holder target.
    pub fn target(&self) -> TerminalTarget {
        TerminalTarget {
            terminal_id: self.spawned.terminal_id.clone(),
            terminal_incarnation: self.spawned.terminal_incarnation,
            process_incarnation: Some(self.spawned.process_incarnation),
        }
    }

    /// Receives holder messages until `done` holds, collecting output; bounded.
    ///
    /// # Errors
    ///
    /// Fails when the bound elapses or the link closes.
    pub fn until(
        &mut self,
        mut done: impl FnMut(&Self, Option<&Message>) -> bool,
    ) -> TestResult<Vec<Message>> {
        let deadline = Instant::now().checked_add(BOUND).ok_or("clock overflow")?;
        let mut seen = Vec::new();
        while !done(self, seen.last()) {
            let left = deadline
                .checked_duration_since(Instant::now())
                .ok_or("holder bound elapsed")?;
            let message = self.link.recv_timeout(left)?;
            if let Message::Output(output) = &message {
                self.output.extend_from_slice(&output.bytes);
            }
            seen.push(message);
        }
        Ok(seen)
    }

    /// Writes every admitted entry: dispatch, holder write, acknowledgement, until idle.
    ///
    /// # Errors
    ///
    /// Fails when the sequencer or the holder link fails.
    pub fn drain(
        &mut self,
        sequencer: &mut Sequencer,
        view: &View<'_>,
    ) -> TestResult<Vec<Receipt>> {
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
            let acked = self.until(|_, last| matches!(last, Some(Message::InputWritten(ack)) if ack.sequence == sequence))?;
            if let Some(Message::InputWritten(ack)) = acked.last() {
                changed.extend(sequencer.acknowledge(ack)?);
            }
        }
    }
}

/// A sequencer live with the program's process incarnation.
///
/// # Errors
///
/// Fails when the fixture target is rejected.
pub fn ready(fixture: &Fixture, pty: &Pty) -> TestResult<Sequencer> {
    let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;
    sequencer.stream_ready(
        process(pty.spawned.process_incarnation.get())?,
        WriteSequence::new(0),
    );
    Ok(sequencer)
}
