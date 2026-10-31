//! The terminals one Windows holder owns, independent of any core connection.
//!
//! Each terminal is a [`ConptyTerminal`] plus the evidence the holder wire reports for
//! it. A pump thread per terminal streams its output, write acknowledgements and exit to
//! whichever core link is attached; with no link attached the terminal keeps running and
//! its output stays readable through `Tail` within the output bound.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use ovis_terminal_protocol::domain::conpty::{ConptyGrid, ConptyLaunch};
use ovis_terminal_protocol::domain::windows_service::HolderPlacement;
use tokio::sync::mpsc::UnboundedSender;

use super::link::LinkError;
use crate::protocol::{
    Exit, HeldTerminal, Message, ProcessIncarnation, ProcessStartToken, Spawn, SpawnFailed,
    SpawnFailureKind, SpawnFailureReason, Spawned, TerminalId, TerminalTarget,
};
use crate::windows::model::{InputLimits, InputSequence};
use crate::windows::{ConptyError, ConptyTerminal, TerminalLimits};

mod control;
mod pump;

/// Input bounds of every terminal, reported in `InputRejected`.
pub(super) const INPUT_BYTES: usize = 16 << 20;
pub(super) const INPUT_ENTRIES: usize = 4096;

pub(super) type Outbox = UnboundedSender<Message>;

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug)]
struct Pending {
    queued: InputSequence,
    sequence: u64,
    grant_epoch: u64,
    fence_epoch: u64,
    bytes: u64,
}

#[derive(Debug, Default)]
struct Evidence {
    output: u64,
    input_sequence: u64,
    fence: u64,
    pending: VecDeque<Pending>,
    exit: Option<Exit>,
}

#[derive(Debug)]
pub(super) struct Entry {
    target: TerminalTarget,
    pid: u32,
    start_token: ProcessStartToken,
    terminal: ConptyTerminal,
    evidence: Mutex<Evidence>,
}

impl Entry {
    fn held(&self) -> HeldTerminal {
        let evidence = lock(&self.evidence);
        HeldTerminal {
            terminal_id: self.target.terminal_id.clone(),
            terminal_incarnation: self.target.terminal_incarnation,
            process_incarnation: self.target.process_incarnation,
            pid: Some(self.pid),
            start_token: Some(self.start_token.clone()),
            output_offset: evidence.output,
            input_sequence: evidence.input_sequence,
            exit: evidence.exit.clone(),
        }
    }
}

/// Every terminal of one holder, and the core link currently attached, if any.
#[derive(Debug)]
pub(super) struct Table {
    placement: HolderPlacement,
    entries: Mutex<BTreeMap<TerminalId, Arc<Entry>>>,
    link: Mutex<Option<Outbox>>,
    last_process: Mutex<u64>,
    last_ack: Mutex<u64>,
    draining: AtomicBool,
    pumps: Mutex<Vec<JoinHandle<()>>>,
}

fn failure(spawn: &Spawn, reason: SpawnFailureReason, hint: &str) -> Message {
    let kind = match reason {
        SpawnFailureReason::EmptyArgv
        | SpawnFailureReason::InvalidGrid
        | SpawnFailureReason::CwdUnavailable
        | SpawnFailureReason::CwdNotDirectory
        | SpawnFailureReason::ExecutableRejected => SpawnFailureKind::InvalidRequest,
        SpawnFailureReason::HolderDraining => SpawnFailureKind::LimitExceeded,
        _ => SpawnFailureKind::Unavailable,
    };
    Message::SpawnFailed(SpawnFailed {
        terminal_id: spawn.terminal_id.clone(),
        terminal_incarnation: spawn.terminal_incarnation,
        kind,
        reason,
        hint: hint.to_owned(),
        os_error: None,
    })
}

const fn refused_reason(error: &ConptyError) -> SpawnFailureReason {
    match error {
        ConptyError::Launch(_) => SpawnFailureReason::EmptyArgv,
        ConptyError::CwdUnavailable { .. } => SpawnFailureReason::CwdUnavailable,
        ConptyError::PseudoconsoleUnavailable { .. } => SpawnFailureReason::PtyUnavailable,
        ConptyError::ProgramUnavailable { .. } => SpawnFailureReason::ExecutableRejected,
        ConptyError::WorkerUnavailable { .. } => SpawnFailureReason::WorkerUnavailable,
        ConptyError::Ownership { .. }
        | ConptyError::ProcessExited
        | ConptyError::Resize { .. }
        | ConptyError::Kill { .. }
        | ConptyError::Output(_)
        | ConptyError::WorkerPanicked { .. } => SpawnFailureReason::ExecFailed,
    }
}

fn launch(spawn: &Spawn) -> Result<ConptyLaunch, SpawnFailureReason> {
    let (cols, rows) = spawn.geometry;
    let grid = ConptyGrid::new(cols, rows).map_err(|_| SpawnFailureReason::InvalidGrid)?;
    ConptyLaunch::new(spawn.argv.clone(), spawn.cwd.clone(), grid)
        .and_then(|launch| launch.with_env(spawn.env.clone()))
        .map_err(|_| SpawnFailureReason::EmptyArgv)
}

impl Table {
    pub(super) fn new(placement: HolderPlacement) -> Arc<Self> {
        Arc::new(Self {
            placement,
            entries: Mutex::new(BTreeMap::new()),
            link: Mutex::new(None),
            last_process: Mutex::new(0),
            last_ack: Mutex::new(0),
            draining: AtomicBool::new(false),
            pumps: Mutex::new(Vec::new()),
        })
    }

    /// Replaces the attached link; the previous core's link closes.
    pub(super) fn attach(&self, outbox: Outbox) {
        *lock(&self.link) = Some(outbox);
    }

    /// Detaches `outbox` if it is still the attached link.
    pub(super) fn detach(&self, outbox: &Outbox) {
        let mut link = lock(&self.link);
        if link
            .as_ref()
            .is_some_and(|current| current.same_channel(outbox))
        {
            *link = None;
        }
    }

    /// The adoption snapshot, in terminal-identity order.
    pub(super) fn held(&self) -> Vec<HeldTerminal> {
        lock(&self.entries)
            .values()
            .map(|entry| entry.held())
            .collect()
    }

    fn publish(&self, message: Message) {
        let mut link = lock(&self.link);
        if link
            .as_ref()
            .is_some_and(|outbox| outbox.send(message).is_err())
        {
            *link = None;
        }
    }

    fn next_ack(&self) -> u64 {
        let mut last = lock(&self.last_ack);
        *last = last.saturating_add(1);
        *last
    }

    /// Starts a program for `spawn`, answering `Spawned` or a typed `SpawnFailed`.
    pub(super) fn spawn(self: &Arc<Self>, spawn: &Spawn) -> Result<(), LinkError> {
        if lock(&self.entries).contains_key(&spawn.terminal_id) {
            return Err(LinkError::DuplicateTerminal {
                terminal_id: spawn.terminal_id.clone(),
            });
        }
        let full = self.placement == HolderPlacement::Dedicated && !lock(&self.entries).is_empty();
        if self.draining.load(Ordering::SeqCst) || full {
            let hint = "this holder accepts no new terminals; start one in another holder";
            self.publish(failure(spawn, SpawnFailureReason::HolderDraining, hint));
            return Ok(());
        }
        let started = launch(spawn).and_then(|launch| {
            let limits = TerminalLimits {
                input: InputLimits::new(INPUT_BYTES, INPUT_ENTRIES),
                ..TerminalLimits::DEFAULT
            };
            ConptyTerminal::spawn(&launch, limits).map_err(|error| refused_reason(&error))
        });
        let terminal = match started {
            Ok(terminal) => terminal,
            Err(reason) => {
                self.publish(failure(
                    spawn,
                    reason,
                    "check the program, directory and grid",
                ));
                return Ok(());
            }
        };
        let process = {
            let mut last = lock(&self.last_process);
            *last = last.checked_add(1).ok_or(LinkError::CounterExhausted)?;
            ProcessIncarnation::new(*last)
        };
        let entry = Arc::new(Entry {
            target: TerminalTarget {
                terminal_id: spawn.terminal_id.clone(),
                terminal_incarnation: spawn.terminal_incarnation,
                process_incarnation: Some(process),
            },
            pid: terminal.pid(),
            start_token: terminal.start_token().clone(),
            terminal,
            evidence: Mutex::new(Evidence::default()),
        });
        self.publish(Message::Spawned(Spawned {
            terminal_id: spawn.terminal_id.clone(),
            terminal_incarnation: spawn.terminal_incarnation,
            process_incarnation: process,
            pid: entry.pid,
            start_token: entry.start_token.clone(),
        }));
        lock(&self.entries).insert(spawn.terminal_id.clone(), Arc::clone(&entry));
        let pump = pump::start(Arc::clone(self), entry).map_err(LinkError::Io)?;
        lock(&self.pumps).push(pump);
        Ok(())
    }

    /// Stops new spawns while every owned terminal keeps running.
    pub(super) fn begin_drain(&self) {
        self.draining.store(true, Ordering::SeqCst);
    }

    /// Whether the holder owns no terminal.
    pub(super) fn is_empty(&self) -> bool {
        lock(&self.entries).is_empty()
    }

    /// Ends every program, joins every pump and releases every terminal.
    ///
    /// Every terminal is attempted; the first kill failure is reported after all of them,
    /// and a pump that panicked is reported as [`LinkError::PumpPanicked`].
    pub(super) fn stop(&self) -> Result<(), LinkError> {
        self.draining.store(true, Ordering::SeqCst);
        let entries: Vec<Arc<Entry>> = lock(&self.entries).values().cloned().collect();
        let mut first_failure = None;
        for entry in &entries {
            if let Err(error) = entry.terminal.kill() {
                first_failure.get_or_insert(error);
            }
        }
        let pumps = std::mem::take(&mut *lock(&self.pumps));
        let panicked = pumps
            .into_iter()
            .map(JoinHandle::join)
            .filter(Result::is_err)
            .count();
        lock(&self.entries).clear();
        if let Some(error) = first_failure {
            return Err(error.into());
        }
        match panicked {
            0 => Ok(()),
            count => Err(LinkError::PumpPanicked { count }),
        }
    }
}
