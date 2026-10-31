//! Core requests against held terminals: input, resize, signal, tail, forget, drain.
//!
//! Resize and signal act on the PTY and process directly and never wait behind
//! queued input, so a program that stops reading stays controllable.

use std::sync::{Arc, PoisonError};

use super::Holder;
use super::admission::admit_input;
use super::signalling::{SignalOutcome, signal};
use super::state::HolderState;
use super::workers::join_outcome;
use crate::process::{ProcessId, SignalScope};
use crate::protocol::{
    Forget, Input, Resize, Signal, SignalKind, Tail, TailRequest, TerminalTarget,
};
use crate::unix::error::HolderError;
use crate::unix::io_error::TerminalIoError;
use crate::unix::join_report::{JoinOutcome, ShutdownReport, WorkerRole, WorkersJoined};
use crate::unix::launch::Grid;
use crate::unix::link::LinkClosed;

impl Holder {
    /// Queues input for the PTY; `InputWritten` later reports the bytes actually written.
    ///
    /// A request that would exceed the byte or entry bound is rejected before any
    /// of its bytes are queued, and `InputRejected` (kind, hint, bounds) is
    /// published to the adopted core; its sequence is not consumed.
    ///
    /// # Errors
    ///
    /// Returns [`HolderError::InputOverflow`] when the bytes do not fit the
    /// per-terminal bound, [`HolderError::SequenceRegression`] for a sequence that
    /// does not advance, [`HolderError::StaleTarget`] when the grant names another
    /// process incarnation, and [`HolderError::ProcessExited`] after exit.
    pub fn input(&self, input: Input) -> Result<(), HolderError> {
        self.shared.with_state(|state| admit_input(state, input))
    }

    /// Unexpected I/O and process-observation failures of a held terminal, with
    /// their sources; the exit record carries the first one in wire form.
    ///
    /// # Errors
    ///
    /// Returns a target error when the terminal is not held under `target`.
    pub fn io_errors(
        &self,
        target: &TerminalTarget,
    ) -> Result<Vec<Arc<TerminalIoError>>, HolderError> {
        self.shared
            .with_state(|state| Ok(state.terminal_mut(target)?.io_errors.clone()))
    }

    /// Applies a new grid immediately, independent of queued input.
    ///
    /// # Errors
    ///
    /// Returns [`HolderError::InvalidGeometry`], [`HolderError::StaleGeometry`]
    /// for an epoch older than the last applied one, [`HolderError::ProcessExited`],
    /// or [`HolderError::Resize`] when the PTY rejects it.
    pub fn resize(&self, resize: &Resize) -> Result<(), HolderError> {
        let (cols, rows) = resize.geometry;
        let grid =
            Grid::new(cols, rows).map_err(|_| HolderError::InvalidGeometry { cols, rows })?;
        self.shared.with_state(|state| {
            let terminal = state.terminal_mut(&resize.target)?;
            if let Some(applied) = terminal
                .geometry_epoch
                .filter(|applied| resize.geometry_epoch < *applied)
            {
                return Err(HolderError::StaleGeometry {
                    epoch: resize.geometry_epoch,
                    applied,
                });
            }
            let live = terminal.live.as_ref().ok_or(HolderError::ProcessExited)?;
            live.master
                .resize(grid.pty_size())
                .map_err(|error| HolderError::Resize {
                    source: error.into(),
                })?;
            terminal.geometry_epoch = Some(resize.geometry_epoch);
            Ok(())
        })
    }

    /// Delivers a signal; delivery is not evidence that the process stopped.
    ///
    /// Interrupt reaches the PTY's foreground process group, like a terminal
    /// interrupt key; terminate and kill reach the launched process's group.
    ///
    /// # Errors
    ///
    /// Returns [`HolderError::StaleTarget`] unless the target names the current
    /// process incarnation, [`HolderError::ProcessExited`], or
    /// [`HolderError::Signal`] when the kernel refuses delivery.
    pub fn signal(&self, request: &Signal) -> Result<(), HolderError> {
        if request.target.process_incarnation.is_none() {
            return Err(HolderError::StaleTarget {
                terminal_id: request.target.terminal_id.clone(),
            });
        }
        self.shared.with_state(|state| {
            let terminal = state.terminal_mut(&request.target)?;
            let live = terminal.live.as_ref().ok_or(HolderError::ProcessExited)?;
            let group = match request.signal {
                SignalKind::Interrupt => live
                    .master
                    .process_group_leader()
                    .and_then(|leader| u32::try_from(leader).ok())
                    .map_or(live.pid, ProcessId::new),
                SignalKind::Terminate | SignalKind::Kill => live.pid,
            };
            match signal(terminal, SignalScope::ProcessGroup(group), request.signal)? {
                SignalOutcome::Delivered => Ok(()),
                SignalOutcome::AlreadyExited => Err(HolderError::ProcessExited),
            }
        })
    }

    /// Returns retained raw output from an offset, with `gap` when it was evicted.
    ///
    /// # Errors
    ///
    /// Returns a target error or [`HolderError::OffsetBeyondHead`].
    pub fn tail(&self, request: &TailRequest) -> Result<Tail, HolderError> {
        self.shared.with_state(|state| {
            let terminal = state.terminal_mut(&request.target)?;
            let slice = terminal.tail.read_from(request.from_offset)?;
            Ok(Tail {
                target: terminal.target(),
                from_offset: slice.from,
                through_offset: slice.through,
                bytes: slice.bytes,
                gap: slice.gap,
            })
        })
    }

    /// Removes an exited terminal once the core acknowledged output through its exit.
    ///
    /// # Errors
    ///
    /// Returns [`HolderError::Adoption`] when no exit is retained for the target
    /// or the acknowledgement ends before the final offset.
    pub fn forget(&self, forget: &Forget) -> Result<(), HolderError> {
        self.shared.with_state(|state| {
            let mut table = state.adoption_table()?;
            // An absent acknowledgement covers no output.
            let acknowledged = forget.acknowledged_final_offset.unwrap_or(0);
            table.acknowledge_exit(&forget.target, acknowledged)?;
            state.terminals.remove(&forget.target.terminal_id);
            Ok::<(), HolderError>(())
        })
    }

    /// Stops accepting spawns; held terminals keep running and reporting.
    pub fn begin_drain(&self) {
        self.shared.with_state(|state| state.draining = true);
    }

    /// Reports whether the holder retains no terminal records.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.shared.with_state(|state| state.terminals.is_empty())
    }

    /// Kills every live process group, closes the core link and joins all workers.
    ///
    /// The report is built only from actual `JoinHandle::join` results: a worker
    /// whose handle was never registered is `NotRegistered`, never `Joined`.
    /// Already-exited targets are counted separately and are never signalled.
    ///
    /// # Errors
    ///
    /// Returns [`HolderError::Signal`] when a live process group could not be
    /// killed, or a process-observation error when incarnation/exit evidence
    /// could not be read; its workers are then not joined.
    pub fn shutdown(&self) -> Result<ShutdownReport, HolderError> {
        let already_exited = self.shared.with_state(kill_all)?;
        let (running, mut outcomes, started) = {
            let mut workers = self
                .shared
                .workers
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            (
                std::mem::take(&mut workers.running),
                std::mem::take(&mut workers.outcomes),
                std::mem::take(&mut workers.started),
            )
        };
        for (terminal_id, role, handle) in running {
            outcomes.insert((terminal_id, role), join_outcome(handle));
        }
        let mut report = ShutdownReport {
            already_exited,
            ..ShutdownReport::default()
        };
        for terminal_id in started {
            let [waiter, pump, writer] = WorkerRole::ALL.map(|role| {
                outcomes
                    .get(&(terminal_id.clone(), role))
                    .copied()
                    .unwrap_or(JoinOutcome::NotRegistered)
            });
            report.terminals.insert(
                terminal_id,
                WorkersJoined {
                    waiter,
                    pump,
                    writer,
                },
            );
        }
        Ok(report)
    }
}

fn kill_all(state: &mut HolderState) -> Result<usize, HolderError> {
    state.draining = true;
    state.detach(LinkClosed::HolderStopped);
    let mut already_exited = 0_usize;
    for terminal in state.terminals.values() {
        let Some(live) = &terminal.live else {
            already_exited = already_exited.saturating_add(1);
            continue;
        };
        match signal(
            terminal,
            SignalScope::ProcessGroup(live.pid),
            SignalKind::Kill,
        )? {
            SignalOutcome::Delivered => {}
            SignalOutcome::AlreadyExited => {
                already_exited = already_exited.saturating_add(1);
            }
        }
    }
    Ok(already_exited)
}
