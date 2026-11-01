//! The measured run and its disruptions: signalled control crashes, planned handovers,
//! observer interruptions, stalled viewers and, after the run, the internal
//! shared/dedicated holder-loss probe.

use std::io::Write as _;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use ovis_terminal_protocol::domain::recovery::Survival;

use super::drive::{DEDICATED, Run, SHARED};
use super::error::SoakError;
use super::plan::Kind;
use super::verdict::Probe;
use super::wire::{BootMode, Hello, Ready};

const TICK: Duration = Duration::from_secs(1);

/// `survival` or `survival:reason`, from the record's own JSON.
pub(super) fn survival_name(survival: &Survival) -> String {
    let value = serde_json::to_value(survival).unwrap_or_default();
    let kind = value
        .get("survival")
        .and_then(|v| v.as_str())
        .unwrap_or("unreadable");
    value
        .get("reason")
        .and_then(|v| v.as_str())
        .map_or_else(|| kind.to_owned(), |reason| format!("{kind}:{reason}"))
}

impl Run<'_> {
    pub(super) fn crash(&mut self, mode: BootMode) -> Result<Box<Ready>, SoakError> {
        let control = self.control.take().ok_or(SoakError::ControlGone {
            waiting_for: "a control boot to crash",
        })?;
        self.close_observer()?;
        self.stalled.clear();
        control.crash()?;
        let (downtime, ready) = self.boot(mode)?;
        self.downtimes_ms.push((
            Kind::CoreCrash,
            u64::try_from(downtime.as_millis()).unwrap_or(u64::MAX),
        ));
        Ok(ready)
    }

    pub(super) fn handover(&mut self) -> Result<Box<Ready>, SoakError> {
        let mut control = self.control.take().ok_or(SoakError::ControlGone {
            waiting_for: "a control boot to hand over",
        })?;
        let asked = Instant::now();
        control.command("handover", true)?;
        let code = control.exit()?;
        self.close_observer()?;
        self.stalled.clear();
        self.planned_exits.push(code);
        let (_, ready) = self.boot(BootMode::Handover)?;
        let downtime = u64::try_from(asked.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.downtimes_ms.push((Kind::PlannedRestart, downtime));
        Ok(ready)
    }

    fn stall(&mut self) -> Result<(), SoakError> {
        let socket = self
            .control
            .as_ref()
            .map(|control| control.socket.clone())
            .ok_or(SoakError::ControlGone {
                waiting_for: "a control boot to stall a viewer on",
            })?;
        let mut stream =
            UnixStream::connect(&socket).map_err(SoakError::io("connect a stalled viewer"))?;
        let hello = Hello {
            viewer: format!("stalled-{}", self.stalled.len().saturating_add(1)),
            reads: false,
            from_seq: None,
        };
        let mut line = serde_json::to_string(&hello)?;
        line.push('\n');
        stream
            .write_all(line.as_bytes())
            .map_err(SoakError::io("send a stalled viewer hello"))?;
        self.stalled.push(stream);
        Ok(())
    }

    fn disrupt(&mut self, kind: Kind) -> Result<(), SoakError> {
        match kind {
            Kind::CoreCrash => self.crash(BootMode::Crash).map(drop)?,
            Kind::PlannedRestart => self.handover().map(drop)?,
            Kind::RemoteInterruption => self.interrupt()?,
            Kind::StalledViewer => self.stall()?,
        }
        self.count(kind);
        Ok(())
    }

    /// The continuous run; returns how long it actually ran.
    pub(super) fn measure(&mut self) -> Result<Duration, SoakError> {
        self.boot(BootMode::Fresh)?;
        let started = Instant::now();
        let end = started.checked_add(self.plan.duration).unwrap_or(started);
        let mut next_tick = started;
        let schedule = self.plan.disruptions.clone();
        let mut pending = schedule.iter().peekable();
        while Instant::now() < end {
            let now = Instant::now();
            let due = pending
                .peek()
                .and_then(|next| started.checked_add(next.at))
                .unwrap_or(end);
            if due <= now {
                if let Some(next) = pending.next() {
                    self.disrupt(next.kind)?;
                }
                continue;
            }
            if now >= next_tick {
                self.tick(started.elapsed())?;
                next_tick = next_tick.checked_add(TICK).unwrap_or(now).max(now);
            }
            self.listen(next_tick.min(due).min(end))?;
        }
        Ok(started.elapsed())
    }

    /// Kills the dedicated holder alone, then crashes the control process: only the
    /// dedicated terminal may become unknown, every shared one must survive unchanged.
    pub(super) fn probe(&mut self) -> Result<Probe, SoakError> {
        let at = self
            .holders
            .iter()
            .position(|holder| holder.session == DEDICATED)
            .ok_or(SoakError::Usage {
                reason: "the holder-loss probe needs a dedicated holder (two terminals or more)",
            })?;
        let last = self.plan.terminals.saturating_sub(1);
        self.tally.excused = Some(last);
        self.holders.remove(at).kill()?;
        let changes = self.tally.identity_changes;
        let ready = self.crash(BootMode::Crash)?;
        let terminals = ready
            .recovery
            .as_ref()
            .map_or(&[][..], |recovery| recovery.terminals());
        let dedicated = terminals.get(last).map_or_else(
            || "missing".to_owned(),
            |recovered| survival_name(&recovered.survival),
        );
        let shared_survived = terminals
            .iter()
            .take(last)
            .filter(|recovered| recovered.survival == (Survival::Survived {}))
            .count();
        Ok(Probe {
            ran: true,
            dedicated,
            shared_survived: u64::try_from(shared_survived).unwrap_or(u64::MAX),
            shared_expected: u64::try_from(last).unwrap_or(u64::MAX),
            shared_identities_unchanged: self.tally.identity_changes == changes,
        })
    }

    /// Stops what this run started, through the binary's own `service stop`; every step
    /// runs and the first failure is returned.
    pub(super) fn teardown(&mut self) -> Result<(), SoakError> {
        let mut first: Option<SoakError> = None;
        let mut keep = |result: Result<(), SoakError>| {
            if let Err(error) = result
                && first.is_none()
            {
                first = Some(error);
            }
        };
        if let Some(mut control) = self.control.take() {
            let stopped = control
                .command("stop", true)
                .and_then(|()| control.exit().map(drop));
            keep(stopped);
        }
        keep(self.close_observer());
        self.stalled.clear();
        for holder in std::mem::take(&mut self.holders) {
            let stop = self
                .env
                .cli(&["service", "stop", "--session", &holder.session]);
            keep(stop.map(|run| self.cli.push(run)));
            keep(holder.reap().map(drop));
        }
        let status = self.env.cli(&["service", "status", "--session", SHARED]);
        keep(status.map(|run| self.cli.push(run)));
        keep(
            std::fs::remove_dir_all(&self.plan.root).map_err(SoakError::io("remove the run root")),
        );
        first.map_or(Ok(()), Err)
    }
}
