//! The failure fixture `holder-tail-gap-and-lost-receipt`, against the real holders of
//! the binary under test: terminal 0 keeps one stale checkpoint while its holder's tail
//! rolls past it, input is sent with a durable intent and its receipt is never committed,
//! and the control process is killed with `SIGKILL`. The fixture passes only when recovery reports a
//! gap and an inexact screen, the input is `outcome_unknown`, and nothing resends it.

use std::time::{Duration, Instant};

use ovis_terminal_holder::unix::HolderLimits;
use ovis_terminal_protocol::domain::recovery::OutputGap;
use ovis_terminal_protocol::domain::restart::InterruptedOutcome;
use ovis_terminal_protocol::schema::scalar::{ByteOffset, Count};
use serde::Serialize;

use super::drive::Run;
use super::error::SoakError;
use super::wire::{BootMode, Event, Line, Ready};

/// Bound on each fixture stage.
const STAGE: Duration = Duration::from_secs(120);
/// Output beyond the holder's retained tail that guarantees the checkpoint is not covered.
const MARGIN: u64 = 64 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(super) struct FixtureOutcome {
    pub(super) checkpoint_offset: u64,
    pub(super) offset_at_crash: u64,
    pub(super) screen_exact: Option<bool>,
    pub(super) gaps: Vec<OutputGap>,
    pub(super) op_id: String,
    pub(super) outcome: String,
    pub(super) input_sequence_after_crash: Option<u64>,
    pub(super) input_sequence_after_handover: Option<u64>,
    pub(super) resent: bool,
}

impl FixtureOutcome {
    /// Exact recovery or a resend is never a pass, whatever else held.
    pub(super) fn passed(&self) -> bool {
        self.screen_exact == Some(false)
            && !self.gaps.is_empty()
            && self.outcome == "outcome_unknown"
            && !self.resent
    }
}

fn outcome_name(outcome: &InterruptedOutcome) -> String {
    serde_json::to_value(outcome)
        .ok()
        .and_then(|value| value.get("outcome")?.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn input_sequence(ready: &Ready) -> Option<u64> {
    ready.terminals.first()?.input_sequence.map(Count::get)
}

impl Run<'_> {
    /// Observes until `pick` accepts an event, at most [`STAGE`].
    fn await_event<T>(&mut self, pick: impl Fn(&Event) -> Option<T>) -> Result<T, SoakError> {
        let deadline = Instant::now()
            .checked_add(STAGE)
            .unwrap_or_else(Instant::now);
        loop {
            let line = self.observer()?.next(deadline)?.ok_or(SoakError::Timeout {
                waiting_for: "fixture stage event",
                bound_ms: u64::try_from(STAGE.as_millis()).unwrap_or(u64::MAX),
            })?;
            self.observe(&line);
            if let Line::Event { event, .. } = &line
                && let Some(found) = pick(event)
            {
                return Ok(found);
            }
        }
    }

    pub(super) fn fixture(&mut self) -> Result<FixtureOutcome, SoakError> {
        self.boot(BootMode::Fresh)?;
        let checkpoint_offset = self.await_event(|event| match event {
            Event::Checkpointed { offset, .. } => Some(offset.get()),
            _ => None,
        })?;
        let mut outcome = FixtureOutcome {
            checkpoint_offset,
            ..FixtureOutcome::default()
        };
        self.control
            .as_mut()
            .ok_or(SoakError::ControlGone {
                waiting_for: "the fixture boot",
            })?
            .command("lose-receipt", false)?;
        outcome.op_id = self.await_event(|event| match event {
            Event::InputSent { op_id, .. } => Some(op_id.clone()),
            _ => None,
        })?;
        let tail = u64::try_from(HolderLimits::DEFAULT.tail_bytes).unwrap_or(u64::MAX);
        let wanted = outcome
            .checkpoint_offset
            .saturating_add(tail)
            .saturating_add(MARGIN);
        outcome.offset_at_crash = self.await_event(|event| match event {
            Event::Progress { offsets, .. } => offsets
                .first()
                .copied()
                .flatten()
                .map(ByteOffset::get)
                .filter(|offset| *offset >= wanted),
            _ => None,
        })?;
        let sent_before = self.tally.inputs_sent;
        let crashed = self.crash(BootMode::Crash)?;
        let first = crashed
            .recovery
            .as_ref()
            .and_then(|recovery| recovery.terminals().first());
        let screen = first.and_then(|recovered| recovered.screen.as_ref());
        outcome.screen_exact = screen.map(|screen| screen.exact);
        outcome.gaps = screen.map(|screen| screen.gaps.clone()).unwrap_or_default();
        outcome.outcome = crashed
            .interrupted
            .iter()
            .find(|op| op.op_id.as_str() == outcome.op_id)
            .map_or_else(|| "missing".to_owned(), |op| outcome_name(&op.outcome));
        outcome.input_sequence_after_crash = input_sequence(&crashed);
        let handed = self.handover()?;
        outcome.input_sequence_after_handover = input_sequence(&handed);
        outcome.resent = self.tally.inputs_sent > sent_before
            || outcome
                .input_sequence_after_crash
                .is_some_and(|seq| seq > 1)
            || outcome.input_sequence_after_handover > outcome.input_sequence_after_crash;
        Ok(outcome)
    }
}
