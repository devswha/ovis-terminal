//! Crash recovery and host-reboot records (plan todo 49).
//!
//! A control core that died without a handover, or a host that rebooted, leaves
//! terminal metadata behind. These records say, per terminal, what independent host
//! boot and process-incarnation evidence proves about the program ([`Survival`]),
//! which output the restored screen was rebuilt from and whether it is exact
//! ([`ScreenRecovery`]), and which durable incidents were recorded. Survival is never
//! claimed from metadata alone: inconclusive evidence is [`Survival::Unknown`], which
//! forbids an automatic relaunch.
//!
//! Holder placement (shared per session and build, or the dedicated test placement)
//! is internal to the runtime and is not part of any record here or of `terminal.run`.

use serde::{Deserialize, Serialize};

use super::restart::identity::Cause;
use crate::schema::ids::{RuntimeLifetimeId, ServerBootId, TerminalId};
use crate::schema::methods::server::RecoveryEvidence;
use crate::schema::records::TerminalOrigin;
use crate::schema::scalar::{ByteOffset, Incarnation, TimestampMs};

#[cfg(test)]
mod tests;

/// Why the evidence could not settle whether a program survived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum UnknownReason {
    /// The operating system's boot identity could not be read.
    BootEvidenceUnreadable,
    /// The holder group that owned the program did not answer adoption.
    HolderUnreachable,
    /// The holder answered, but the process start record could not be read.
    ProcessEvidenceUnreadable,
}

/// Why a program is proven not to have survived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LossReason {
    /// The holder retained its exit.
    Exited,
    /// Another process identity runs under the recorded PID or terminal.
    Replaced,
    /// The reachable holder no longer holds the terminal.
    NotHeld,
    /// The operating system runs no process with the recorded start token.
    Gone,
}

/// What independent evidence proves about one recorded program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "survival", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Survival {
    /// Same host boot, the holder still owns it, and the operating system runs it with
    /// the recorded PID and start token.
    Survived {},
    /// The host rebooted: only metadata was restored and no process survived.
    RestoredMetadata {},
    /// The program was explicitly relaunched under a new process incarnation.
    Relaunched {
        /// The new process incarnation.
        process_incarnation: Incarnation,
    },
    /// The program is proven gone on this host boot.
    Lost {
        /// Why.
        reason: LossReason,
    },
    /// The evidence is inconclusive; nothing is relaunched automatically.
    Unknown {
        /// What could not be established.
        reason: UnknownReason,
    },
}

impl Survival {
    /// Whether the program is proven gone, so an explicit relaunch cannot duplicate it.
    #[must_use]
    pub const fn may_relaunch(&self) -> bool {
        matches!(self, Self::RestoredMetadata {} | Self::Lost { .. })
    }

    /// The terminal record origin this verdict maps to; `None` while no record origin
    /// applies (lost or unknown programs stay `lost` or `recovering`).
    #[must_use]
    pub const fn origin(&self) -> Option<TerminalOrigin> {
        match self {
            Self::Survived {} => Some(TerminalOrigin::Live),
            Self::RestoredMetadata {} => Some(TerminalOrigin::RestoredMetadata),
            Self::Relaunched { .. } => Some(TerminalOrigin::Relaunched),
            Self::Lost { .. } | Self::Unknown { .. } => None,
        }
    }
}

/// Output bytes `[from, to)` that no retained source holds any more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct OutputGap {
    /// First missing offset.
    pub from: ByteOffset,
    /// Offset just past the last missing byte.
    pub to: ByteOffset,
}

impl OutputGap {
    /// The gap `[from, to)`.
    #[must_use]
    pub const fn new(from: ByteOffset, to: ByteOffset) -> Self {
        Self { from, to }
    }
}

/// How a terminal's screen was rebuilt after the crash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ScreenRecovery {
    /// Offset of the durable checkpoint the rebuild started from; absent when it
    /// started from an empty screen at offset zero.
    pub checkpoint_offset: Option<ByteOffset>,
    /// First retained byte replayed after the checkpoint.
    pub replayed_from: ByteOffset,
    /// Output head the rebuild reached.
    pub head: ByteOffset,
    /// Output nobody retains between the checkpoint and the head.
    pub gaps: Vec<OutputGap>,
    /// Whether the rebuilt screen equals an engine that saw the whole stream.
    pub exact: bool,
}

impl ScreenRecovery {
    /// A rebuild from `checkpoint_offset` that replayed `[replayed_from, head)`.
    #[must_use]
    pub const fn new(
        (checkpoint_offset, replayed_from, head): (Option<ByteOffset>, ByteOffset, ByteOffset),
        gaps: Vec<OutputGap>,
        exact: bool,
    ) -> Self {
        Self {
            checkpoint_offset,
            replayed_from,
            head,
            gaps,
            exact,
        }
    }
}

/// The recovery outcome of one terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TerminalRecovery {
    /// Terminal identity.
    pub terminal_id: TerminalId,
    /// Terminal incarnation recorded before the crash.
    pub terminal_incarnation: Incarnation,
    /// What the evidence proves about its program.
    pub survival: Survival,
    /// How the screen was rebuilt, for a survivor.
    pub screen: Option<ScreenRecovery>,
}

impl TerminalRecovery {
    /// The verdict `survival` for `terminal_id` at `terminal_incarnation`.
    #[must_use]
    pub fn new(
        (terminal_id, terminal_incarnation): (TerminalId, Incarnation),
        survival: Survival,
        screen: Option<ScreenRecovery>,
    ) -> Self {
        Self {
            terminal_id,
            terminal_incarnation,
            survival,
            screen,
        }
    }
}

/// What a recorded incident is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum IncidentKind {
    /// The control core ended without a planned handover.
    CoreCrash,
    /// The host rebooted since the runtime last recorded its boot.
    HostReboot,
    /// A holder group did not answer adoption.
    HolderUnreachable,
    /// The operating system boot identity could not be read.
    BootEvidenceUnreadable,
    /// The state database failed its integrity check and was quarantined.
    StoreCorrupted,
    /// The state database was written by a newer schema and was preserved.
    StoreUnsupported,
    /// A screen checkpoint failed its checksum or did not decode and was quarantined.
    CheckpointCorrupted,
    /// A screen checkpoint names a format this build cannot read and was quarantined.
    CheckpointUnsupported,
}

/// Identity of one recorded incident; ids sort by recording order.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IncidentId(String);

impl IncidentId {
    /// Wraps an incident id.
    #[must_use]
    pub const fn new(id: String) -> Self {
        Self(id)
    }

    /// The id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One durable incident; reads never clear or expire it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RecoveryIncident {
    /// Incident identity.
    pub incident_id: IncidentId,
    /// What happened.
    pub kind: IncidentKind,
    /// Boot that recorded it.
    pub server_boot_id: ServerBootId,
    /// When it was recorded.
    pub recorded_ms: TimestampMs,
    /// Terminals it affects.
    pub terminals: Vec<TerminalId>,
    /// Where damaged or unsupported material was kept, when any was.
    pub quarantine: Option<String>,
}

impl RecoveryIncident {
    /// Incident `incident_id` of `kind`, recorded by `server_boot_id` at `recorded_ms`.
    #[must_use]
    pub fn new(
        (incident_id, kind): (IncidentId, IncidentKind),
        (server_boot_id, recorded_ms): (ServerBootId, TimestampMs),
        (terminals, quarantine): (Vec<TerminalId>, Option<String>),
    ) -> Self {
        Self {
            incident_id,
            kind,
            server_boot_id,
            recorded_ms,
            terminals,
            quarantine,
        }
    }
}

/// The outcome of one unplanned boot: crash recovery or host-reboot restoration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CrashRecovery {
    /// `core_crash` or `host_reboot`; absent when boot evidence was inconclusive.
    pub cause: Option<Cause>,
    /// Runtime lifetime served after recovery.
    pub runtime_lifetime: RuntimeLifetimeId,
    /// Boot identity that performed the recovery.
    pub server_boot_id: ServerBootId,
    /// Every recorded terminal, with its verdict.
    pub terminals: Vec<TerminalRecovery>,
    /// Incidents recorded by this recovery.
    pub incidents: Vec<IncidentId>,
}

impl CrashRecovery {
    /// A recovery of `cause` served as `server_boot_id` of `runtime_lifetime`.
    #[must_use]
    pub fn new(
        cause: Option<Cause>,
        (runtime_lifetime, server_boot_id): (RuntimeLifetimeId, ServerBootId),
        (terminals, incidents): (Vec<TerminalRecovery>, Vec<IncidentId>),
    ) -> Self {
        Self {
            cause,
            runtime_lifetime,
            server_boot_id,
            terminals,
            incidents,
        }
    }

    /// The `server.recovery` evidence: survivors and relaunched incarnations only;
    /// restored metadata, lost and unknown programs are never listed as restored.
    #[must_use]
    pub fn evidence(&self, recorded_ms: TimestampMs) -> RecoveryEvidence {
        let restored = self
            .terminals
            .iter()
            .filter_map(|terminal| match terminal.survival {
                Survival::Survived {} => Some(terminal.terminal_incarnation),
                Survival::Relaunched {
                    process_incarnation,
                } => Some(process_incarnation),
                Survival::RestoredMetadata {}
                | Survival::Lost { .. }
                | Survival::Unknown { .. } => None,
            })
            .collect();
        RecoveryEvidence {
            server_boot_id: self.server_boot_id.clone(),
            restored,
            gaps: Vec::new(),
            recorded_ms,
        }
    }
}
