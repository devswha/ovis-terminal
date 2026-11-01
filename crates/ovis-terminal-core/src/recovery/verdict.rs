//! The survival verdict: independent boot evidence first, then the holder group's
//! answer, then the operating system's start token for the recorded PID.

use ovis_terminal_holder::protocol::{HeldTerminal, ProcessStartToken, TerminalTarget};
use ovis_terminal_protocol::domain::recovery::{LossReason, Survival, UnknownReason};

use super::BootVerdict;
use super::groups::GroupId;
use super::replay::Fence;

/// A program the durable store recorded as running before the boot.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Recorded {
    /// Terminal, terminal incarnation and process incarnation.
    pub target: TerminalTarget,
    /// The recorded PID.
    pub pid: u32,
    /// The recorded operating-system start token.
    pub start_token: ProcessStartToken,
    /// The holder group that owned it.
    pub group: GroupId,
    /// Side effects the crashed core already performed for it.
    pub fence: Fence,
}

impl Recorded {
    /// `target` ran as `pid` with `start_token` in `group`.
    #[must_use]
    pub fn new(
        target: TerminalTarget,
        (pid, start_token): (u32, ProcessStartToken),
        group: GroupId,
    ) -> Self {
        Self {
            target,
            pid,
            start_token,
            group,
            fence: Fence::new(0, 0),
        }
    }

    /// The same record with the side-effect fence the store kept.
    #[must_use]
    pub const fn with_fence(mut self, fence: Fence) -> Self {
        self.fence = fence;
        self
    }
}

/// What the holder group that owned a program answered at adoption.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum HolderAnswer<'a> {
    /// The group's socket did not answer.
    Unreachable,
    /// The group answered; `None` when it no longer holds the terminal.
    Held(Option<&'a HeldTerminal>),
}

/// What the operating system reports for a PID.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Liveness {
    /// A process runs with this start token.
    Running(ProcessStartToken),
    /// No process runs under the PID.
    Gone,
    /// The process table could not be read.
    Unreadable,
}

/// The evidence gathered for one recorded program.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Evidence<'a> {
    /// Host boot comparison.
    pub boot: BootVerdict,
    /// The owning holder group's answer.
    pub holder: HolderAnswer<'a>,
}

/// Decides what `recorded` is, asking `probe` about the PID only once the boot and the
/// holder already confirm the same identity; PIDs 0 and 1 are never probed.
pub fn verdict(
    evidence: Evidence<'_>,
    recorded: &Recorded,
    probe: impl FnOnce(u32) -> Liveness,
) -> Survival {
    let held = match (evidence.boot, evidence.holder) {
        (BootVerdict::Rebooted, _) => return Survival::RestoredMetadata {},
        (BootVerdict::Inconclusive, _) => {
            return unknown(UnknownReason::BootEvidenceUnreadable);
        }
        (BootVerdict::Same, HolderAnswer::Unreachable) => {
            return unknown(UnknownReason::HolderUnreachable);
        }
        (BootVerdict::Same, HolderAnswer::Held(None)) => return lost(LossReason::NotHeld),
        (BootVerdict::Same, HolderAnswer::Held(Some(held))) => held,
    };
    if held.exit.is_some() {
        return lost(LossReason::Exited);
    }
    let same_identity = held.terminal_incarnation == recorded.target.terminal_incarnation
        && held.process_incarnation == recorded.target.process_incarnation
        && held.pid == Some(recorded.pid)
        && held.start_token.as_ref() == Some(&recorded.start_token);
    if !same_identity || recorded.pid <= 1 {
        return lost(LossReason::Replaced);
    }
    match probe(recorded.pid) {
        Liveness::Running(token) if token == recorded.start_token => Survival::Survived {},
        Liveness::Running(_) => lost(LossReason::Replaced),
        Liveness::Gone => lost(LossReason::Gone),
        Liveness::Unreadable => unknown(UnknownReason::ProcessEvidenceUnreadable),
    }
}

const fn unknown(reason: UnknownReason) -> Survival {
    Survival::Unknown { reason }
}

const fn lost(reason: LossReason) -> Survival {
    Survival::Lost { reason }
}
