//! What the harness concludes from the lines it observes: program identities against
//! the first boot, recovery outcomes per boot, output offsets, queues and control
//! replies.

use ovis_terminal_holder::protocol::ProcessStartToken;
use ovis_terminal_protocol::domain::recovery::Survival;
use ovis_terminal_protocol::schema::ids::ProcessPid;
use ovis_terminal_protocol::schema::scalar::ByteOffset;
use serde::Serialize;

use super::wire::{BootMode, Event, Health, Line, Ready};

/// One recovery boot as the harness judged it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(super) struct RecoveryNote {
    pub(super) boot: u64,
    pub(super) mode: String,
    pub(super) survived: u64,
    pub(super) not_survived: u64,
    pub(super) exact: u64,
    pub(super) inexact: u64,
    pub(super) gaps: u64,
    /// Survivors whose recovery carried no screen record.
    pub(super) unreported_screens: u64,
    pub(super) interrupted: u64,
    pub(super) outcome_unknown: u64,
}

/// A program loss nobody asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct Loss {
    pub(super) boot: u64,
    pub(super) terminal: usize,
    pub(super) what: String,
}

pub(super) const fn mode_name(mode: BootMode) -> &'static str {
    match mode {
        BootMode::Fresh => "fresh",
        BootMode::Crash => "crash",
        BootMode::Handover => "handover",
    }
}

#[derive(Debug, Default, Serialize)]
pub(super) struct Tally {
    #[serde(skip)]
    baseline: Vec<(ProcessPid, ProcessStartToken)>,
    /// A terminal whose loss the holder-loss probe intends.
    #[serde(skip)]
    pub(super) excused: Option<usize>,
    pub(super) last_offsets: Vec<Option<u64>>,
    pub(super) offset_regressions: u64,
    pub(super) losses: Vec<Loss>,
    pub(super) identity_changes: u64,
    pub(super) recoveries: Vec<RecoveryNote>,
    pub(super) max_queued_bytes: u64,
    pub(super) queue_limit_bytes: u64,
    pub(super) viewer_failures: u64,
    pub(super) accept_failures: u64,
    pub(super) viewer_cuts: Vec<String>,
    pub(super) pongs: u64,
    pub(super) control_max_ms: u64,
    pub(super) late_pongs: u64,
    pub(super) inputs_sent: u64,
}

impl Tally {
    fn lose(&mut self, (boot, terminal): (u64, usize), what: String) {
        if self.excused != Some(terminal) {
            self.losses.push(Loss {
                boot,
                terminal,
                what,
            });
        }
    }

    /// Takes the first boot's identities as the baseline; judges every later boot's
    /// recovery against it.
    pub(super) fn ready(&mut self, ready: &Ready) {
        let boot = ready.boot.get();
        if ready.mode == BootMode::Fresh {
            self.baseline = ready
                .terminals
                .iter()
                .map(|identity| (identity.pid, identity.start_token.clone()))
                .collect();
            self.last_offsets = vec![None; self.baseline.len()];
            return;
        }
        let mut note = RecoveryNote {
            boot,
            mode: mode_name(ready.mode).to_owned(),
            interrupted: u64::try_from(ready.interrupted.len()).unwrap_or(u64::MAX),
            outcome_unknown: u64::try_from(
                ready.interrupted.iter().filter(|op| op.uncertain()).count(),
            )
            .unwrap_or(u64::MAX),
            ..RecoveryNote::default()
        };
        let terminals = ready
            .recovery
            .as_ref()
            .map_or(&[][..], |recovery| recovery.terminals());
        for (index, recovered) in terminals.iter().enumerate() {
            if recovered.survival != (Survival::Survived {}) {
                note.not_survived = note.not_survived.saturating_add(1);
                let what = serde_json::to_string(&recovered.survival).unwrap_or_default();
                self.lose((boot, index), what);
                continue;
            }
            note.survived = note.survived.saturating_add(1);
            let now = ready
                .terminals
                .get(index)
                .map(|identity| (identity.pid, identity.start_token.clone()));
            if now.as_ref() != self.baseline.get(index) {
                self.identity_changes = self.identity_changes.saturating_add(1);
                self.lose(
                    (boot, index),
                    "survived under another pid or start token".to_owned(),
                );
            }
            match &recovered.screen {
                None => note.unreported_screens = note.unreported_screens.saturating_add(1),
                Some(screen) if screen.exact => note.exact = note.exact.saturating_add(1),
                Some(_) => note.inexact = note.inexact.saturating_add(1),
            }
            let gaps = recovered
                .screen
                .as_ref()
                .map_or(0, |screen| screen.gaps.len());
            note.gaps = note
                .gaps
                .saturating_add(u64::try_from(gaps).unwrap_or(u64::MAX));
        }
        self.recoveries.push(note);
    }

    fn progress(&mut self, offsets: &[Option<u64>]) {
        for (index, offset) in offsets.iter().enumerate() {
            let Some(offset) = *offset else { continue };
            if let Some(slot) = self.last_offsets.get_mut(index) {
                if slot.is_some_and(|last| offset < last) {
                    self.offset_regressions = self.offset_regressions.saturating_add(1);
                }
                *slot = Some(slot.map_or(offset, |last| last.max(offset)));
            }
        }
    }

    pub(super) fn health(&mut self, health: &Health) {
        self.queue_limit_bytes = health.queue_limit_bytes.get();
        self.accept_failures = self.accept_failures.max(health.accept_failures.get());
        for viewer in &health.viewers {
            self.max_queued_bytes = self.max_queued_bytes.max(viewer.max_queued_bytes.get());
            self.viewer_failures = self.viewer_failures.max(viewer.failures.get());
        }
    }

    /// Accounts one observed line of boot `boot`.
    pub(super) fn line(&mut self, boot: u64, line: &Line) {
        match line {
            Line::Event { event, .. } => match event {
                Event::Ready(ready) => self.ready(ready),
                Event::Progress { offsets, .. } | Event::HandedOver { offsets } => {
                    let plain: Vec<Option<u64>> =
                        offsets.iter().map(|o| o.map(ByteOffset::get)).collect();
                    self.progress(&plain);
                }
                Event::Exited {
                    terminal,
                    final_offset,
                    code,
                    signal,
                } => {
                    let index = usize::try_from(terminal.get()).unwrap_or(usize::MAX);
                    let status = |value: &Option<i32>| {
                        value.map_or_else(|| "none".to_owned(), |v| v.to_string())
                    };
                    let what = format!(
                        "exited at {} code {} signal {}",
                        final_offset.get(),
                        status(code),
                        status(signal)
                    );
                    self.lose((boot, index), what);
                }
                Event::ViewerCut { viewer, .. } => self.viewer_cuts.push(viewer.clone()),
                Event::InputSent { .. } => self.inputs_sent = self.inputs_sent.saturating_add(1),
                Event::Checkpointed { .. } | Event::LinkLost { .. } => {}
            },
            Line::Status { health, .. } => self.health(health),
            Line::Gap { .. } | Line::Pong { .. } => {}
        }
    }

    pub(super) fn pong(&mut self, millis: u64) {
        self.pongs = self.pongs.saturating_add(1);
        self.control_max_ms = self.control_max_ms.max(millis);
    }

    pub(super) const fn late(&mut self) {
        self.late_pongs = self.late_pongs.saturating_add(1);
    }
}
