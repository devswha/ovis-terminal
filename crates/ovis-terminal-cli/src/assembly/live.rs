//! A spawned program incarnation: its screen and attach streams, its input sequence and
//! its geometry controller, all fed from the ordered holder stream.

use std::sync::Arc;

use ovis_terminal_authz::{ResolvedTarget, TargetResource};
use ovis_terminal_core::geometry::{GeometryController, GeometryEpoch};
use ovis_terminal_core::input::encoding::ModeSnapshot;
use ovis_terminal_core::input::sequencer::{Binding, Moment, QueueLimits, Sequencer, View};
use ovis_terminal_core::leases::TerminalLeases;
use ovis_terminal_core::registry::{
    ProcessGeneration, RegistryIdentities, TerminalGeneration, TerminalId,
};
use ovis_terminal_core::screen::{QueryReply, ReplyBatch, Screen, ScreenBounds, ScreenConfig};
use ovis_terminal_core::waits::Waits;
use ovis_terminal_holder::protocol::{Message, Spawned, TerminalTarget};
use ovis_terminal_protocol::schema::common::Geometry as WireGeometry;
use ovis_terminal_protocol::schema::methods::server::WriteSequence;
use ovis_terminal_protocol::schema::scalar::{ByteOffset, TimestampMs};
use ovis_terminal_server::streams::TerminalStreams;
use ovis_terminal_vt::{Geometry, HistoryLimit, VtConfig};

use super::StageSource;
use super::actor::{Runtime, now};

/// Scrollback lines every screen keeps.
const HISTORY_LINES: u32 = 10_000;

/// One program incarnation the server is serving.
#[derive(Debug)]
pub(super) struct Live {
    pub(super) resolved: ResolvedTarget<RegistryIdentities>,
    pub(super) hub: Arc<TerminalStreams>,
    pub(super) sequencer: Sequencer,
    pub(super) geometry: GeometryController,
    pub(super) spawned_ms: TimestampMs,
    pub(super) generation: TerminalGeneration,
    pub(super) ended: bool,
    pub(super) waits: Arc<Waits>,
}

impl Live {
    /// The modes input is encoded against and the moment of the check.
    pub(super) fn view_parts(&self) -> Result<(ModeSnapshot, Moment), StageSource> {
        let screen: &Screen = self.hub.screen();
        let paint = screen.first_paint()?;
        let modes = ModeSnapshot::new(paint.mode_revision, paint.modes);
        let output = ByteOffset::new(screen.live()?.offset.get());
        Ok((modes, Moment::new(now()?, output)))
    }

    /// Hands every admitted entry that passes its write-boundary recheck to the holder.
    pub(super) fn pump(
        &mut self,
        holder: &ovis_terminal_holder::unix::Holder,
    ) -> Result<(), StageSource> {
        loop {
            let (modes, moment) = self.view_parts()?;
            let sequencer = &mut self.sequencer;
            let step = self.hub.with_leases(|leases: &TerminalLeases| {
                sequencer.next(&View::new(leases, &modes, moment))
            })??;
            match step.dispatch {
                Some(input) => holder.input(input)?,
                None => return Ok(()),
            }
        }
    }

    fn replies(&mut self, batch: ReplyBatch) -> Result<(), StageSource> {
        for reply in batch.replies {
            if let QueryReply::Reply { bytes, .. } = reply {
                self.sequencer.admit_reply(bytes)?;
            }
        }
        Ok(())
    }
}

impl Runtime {
    fn spawned(&mut self, spawned: &Spawned) -> Result<(), StageSource> {
        let terminal: TerminalId = spawned.terminal_id.as_str().parse()?;
        let generation = TerminalGeneration::new(spawned.terminal_incarnation.get())?;
        let target = TerminalTarget {
            terminal_id: spawned.terminal_id.clone(),
            terminal_incarnation: spawned.terminal_incarnation,
            process_incarnation: Some(spawned.process_incarnation),
        };
        let (grid, name) = self
            .launches
            .remove(&terminal)
            .unwrap_or((WireGeometry::new(80, 24), None));
        let vt = VtConfig::new(
            Geometry::new(grid.cols, grid.rows)?,
            HistoryLimit::new(HISTORY_LINES),
        );
        let screen = Screen::new(target.clone(), ScreenConfig::new(vt, ScreenBounds::DEFAULT))?;
        let waits = Arc::new(Waits::new(Screen::new(
            target,
            ScreenConfig::new(vt, ScreenBounds::DEFAULT),
        )?));
        let resolved = ResolvedTarget::new(
            self.identity.machine_id(),
            TargetResource::Terminal {
                placement: None,
                terminal,
                incarnation: generation,
            },
        );
        let leases = TerminalLeases::new(Arc::clone(&self.authority), resolved.clone())?;
        let hub = TerminalStreams::new(screen.clone(), leases, self.setup.clone());
        let binding = Binding::new(
            Arc::clone(&self.authority),
            self.identity.runtime_id(),
            resolved.clone(),
        )?;
        let mut sequencer = Sequencer::new(binding, QueueLimits::DEFAULT);
        let process = ProcessGeneration::new(spawned.process_incarnation.get())?;
        sequencer.stream_ready(process, WriteSequence::new(0));
        let geometry = GeometryController::new(terminal, screen, GeometryEpoch::new(0))?;
        self.lane
            .install((terminal, generation), Arc::clone(&hub))?;
        self.directory.spawned((terminal, generation), name);
        let live = Live {
            resolved,
            hub,
            sequencer,
            geometry,
            spawned_ms: now()?,
            generation,
            ended: false,
            waits,
        };
        self.live.insert(terminal, live);
        Ok(())
    }

    /// Applies one committed holder message to the screen, streams and input sequence.
    pub(super) fn follow(&mut self, message: &Message) -> Result<(), StageSource> {
        let terminal: TerminalId = match message {
            Message::Spawned(spawned) => return self.spawned(spawned),
            Message::Output(output) => output.target.terminal_id.as_str().parse()?,
            Message::InputWritten(written) => written.target.terminal_id.as_str().parse()?,
            Message::Exit(exit) => exit.target.terminal_id.as_str().parse()?,
            _ => return Ok(()),
        };
        let Some(live) = self.live.get_mut(&terminal) else {
            return Ok(());
        };
        match message {
            Message::Output(output) => {
                live.waits.ingest(output)?;
                let replies = live.hub.publish(output)?;
                live.replies(replies)?;
            }
            Message::InputWritten(written) => {
                live.sequencer.acknowledge(written)?;
            }
            Message::Exit(exit) => {
                live.waits.finish(exit)?;
                live.hub.finish(exit)?;
                live.sequencer.exited();
                live.ended = true;
                self.directory.exited(terminal);
                return Ok(());
            }
            _ => return Ok(()),
        }
        live.pump(&self.holder)
    }
}
