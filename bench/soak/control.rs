//! The control process the harness crashes and restarts. It adopts the holders the
//! binary under test serves, owns every screen and its checkpoints, and publishes what
//! it knows to viewers. Its stdin carries the harness's commands; end of input stops it.

mod hub;
mod io;
mod links;
mod ops;
mod screens;
mod setup;
mod shutdown;
mod start;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use hub::{Hub, ViewerMsg};
pub(super) use io::Command;
use io::{announce, read_commands, start_links};
use links::Outbound;
use ovis_terminal_core::recovery::CheckpointStore;
use ovis_terminal_holder::protocol::Message;
use ovis_terminal_protocol::schema::scalar::{ByteOffset, Count};
use screens::Screen;
pub(super) use setup::{GroupSpec, Placement, Setup};

use super::error::SoakError;
use super::wire::{BootMode, Event};

const PROGRESS_EVERY: Duration = Duration::from_secs(1);

/// Everything the control loop waits on, on one channel.
#[derive(Debug)]
pub(super) enum Inbound {
    Holder { message: Message },
    LinkLost { group: usize },
    Viewer(ViewerMsg),
    Command(Command),
}

#[derive(Debug)]
struct Control {
    setup: Setup,
    store: CheckpointStore,
    screens: Vec<Option<Screen>>,
    index: BTreeMap<String, usize>,
    links: Vec<Option<Sender<Outbound>>>,
    hub: Hub,
    checkpoints: u64,
    cursor: usize,
}

impl Control {
    fn offsets(&self) -> Vec<Option<ByteOffset>> {
        self.screens
            .iter()
            .map(|screen| screen.as_ref().map(|screen| ByteOffset::new(screen.head())))
            .collect()
    }

    fn lag(&self) -> Vec<u64> {
        self.screens
            .iter()
            .map(|screen| screen.as_ref().map_or(0, Screen::lag))
            .collect()
    }

    fn holder(&mut self, message: Message) -> Result<(), SoakError> {
        match message {
            Message::Output(output) => {
                let index = self.index.get(output.target.terminal_id.as_str()).copied();
                if let Some(screen) = index.and_then(|index| self.screens.get_mut(index)?.as_mut())
                {
                    screen.admit(output.offset, &output.bytes);
                }
                Ok(())
            }
            Message::Exit(exit) => {
                let Some(index) = self.index.get(exit.target.terminal_id.as_str()).copied() else {
                    return Ok(());
                };
                if let Some(slot) = self.screens.get_mut(index) {
                    *slot = None;
                }
                self.hub.publish(Event::Exited {
                    terminal: Count::new(u64::try_from(index).unwrap_or(u64::MAX)),
                    final_offset: ByteOffset::new(exit.final_offset),
                    code: exit.code,
                    signal: exit.signal,
                })
            }
            _ => Ok(()),
        }
    }

    /// Checkpoints the next `count` terminals in turn, so a full round is spread over
    /// the checkpoint period instead of stalling the loop at once.
    fn checkpoint(&mut self, count: usize) -> Result<(), SoakError> {
        let total = self.screens.len();
        for _ in 0..count.min(total) {
            let index = self.cursor;
            self.cursor = self
                .cursor
                .saturating_add(1)
                .checked_rem(total)
                .unwrap_or(0);
            let Some(screen) = self.screens.get_mut(index).and_then(Option::as_mut) else {
                continue;
            };
            let Some(offset) = screen.checkpoint(&self.store)? else {
                continue;
            };
            self.checkpoints = self.checkpoints.saturating_add(1);
            if self.setup.fixture && index == 0 {
                self.hub.publish(Event::Checkpointed {
                    terminal: Count::new(0),
                    offset: ByteOffset::new(offset),
                })?;
            }
        }
        Ok(())
    }

    /// Runs until a command ends the boot; `true` when it handed over.
    fn serve(&mut self, events: &Receiver<Inbound>) -> Result<bool, SoakError> {
        let ticks = self
            .setup
            .checkpoint_every_ms
            .get()
            .checked_div(1000)
            .unwrap_or(1)
            .max(1);
        let per_tick = self
            .screens
            .len()
            .div_ceil(usize::try_from(ticks).unwrap_or(1));
        let mut progress_at = Instant::now();
        loop {
            let now = Instant::now();
            if now >= progress_at {
                self.checkpoint(per_tick)?;
                let offsets = self.offsets();
                let checkpoints = Count::new(self.checkpoints);
                self.hub.publish(Event::Progress {
                    offsets,
                    checkpoints,
                })?;
                progress_at = now.checked_add(PROGRESS_EVERY).unwrap_or(now);
            }
            let wait = progress_at.saturating_duration_since(Instant::now());
            match events.recv_timeout(wait) {
                Ok(Inbound::Holder { message }) => self.holder(message)?,
                Ok(Inbound::LinkLost { group }) => self.hub.publish(Event::LinkLost {
                    group: Count::new(u64::try_from(group).unwrap_or(u64::MAX)),
                })?,
                Ok(Inbound::Viewer(message)) => {
                    let lag = self.lag();
                    self.hub.handle(message, &lag)?;
                }
                Ok(Inbound::Command(Command::LoseReceipt)) => self.lose_receipt()?,
                Ok(Inbound::Command(Command::Stop)) => return Ok(false),
                Ok(Inbound::Command(Command::Handover)) => return Ok(true),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(SoakError::ControlGone {
                        waiting_for: "control inputs",
                    });
                }
            }
        }
    }
}

/// Runs one control boot of the soak rooted at `root`.
pub(super) fn run(root: &Path, (mode, boot): (BootMode, u64)) -> Result<(), SoakError> {
    let setup = Setup::load(root)?;
    let (inbound, events) = std::sync::mpsc::sync_channel(4096);
    let limit = setup.queue_limit_bytes.get();
    let mut hub = Hub::bind(&setup.socket, (inbound.clone(), limit))?;
    announce(&setup.socket)?;
    let begun = start::begin(&setup, (mode, boot))?;
    let commands = inbound.clone();
    let reader = std::thread::Builder::new()
        .name("soak-commands".to_owned())
        .spawn(move || read_commands(&commands))
        .map_err(SoakError::io("start the command reader"))?;
    let (links, threads) = start_links(begun.links, &inbound)?;
    drop(inbound);
    hub.publish(Event::Ready(Box::new(begun.ready)))?;
    let mut screens = begun.screens;
    if setup.fixture
        && let Some(first) = screens.first_mut().and_then(Option::as_mut)
    {
        first.freeze();
    }
    let index = screens
        .iter()
        .enumerate()
        .filter_map(|(at, screen)| {
            Some((screen.as_ref()?.target.terminal_id.as_str().to_owned(), at))
        })
        .collect();
    let store = start::plan(&setup, boot)?.checkpoints();
    let mut control = Control {
        setup,
        store,
        screens,
        index,
        links,
        hub,
        checkpoints: 0,
        cursor: 0,
    };
    let handed_over = control.serve(&events)?;
    if handed_over {
        control.store_all()?;
    }
    control.finish((handed_over, events), (threads, reader))
}
