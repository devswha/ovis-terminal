//! Which identity survives which cause: the published identity transition table.
//!
//! `docs/identity-transitions.md` carries [`markdown`] verbatim. The runtime lifetime
//! survives a control restart because the successor reads it from the persisted restart
//! intent; it is never taken from a cached endpoint descriptor or a hello reply.

use serde::{Deserialize, Serialize};

/// An event that ends or replaces part of the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Cause {
    /// `server.restart`: the control server is replaced on purpose.
    ControlRestart,
    /// The control server died without a handover; its holders kept running.
    CoreCrash,
    /// A holder died; the control server kept running.
    HolderLoss,
    /// A verified build was activated through a control restart.
    Upgrade,
    /// The host rebooted; every process of the old lifetime is gone.
    HostReboot,
}

impl Cause {
    /// Every cause, in table order.
    pub const ALL: [Self; 5] = [
        Self::ControlRestart,
        Self::CoreCrash,
        Self::HolderLoss,
        Self::Upgrade,
        Self::HostReboot,
    ];

    /// The wire token.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::ControlRestart => "control_restart",
            Self::CoreCrash => "core_crash",
            Self::HolderLoss => "holder_loss",
            Self::Upgrade => "upgrade",
            Self::HostReboot => "host_reboot",
        }
    }
}

/// An identity a client may hold across the cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Field {
    /// `machine_id`.
    MachineId,
    /// `runtime_lifetime`.
    RuntimeLifetime,
    /// `server_boot_id`.
    ServerBootId,
    /// `terminal_id`.
    TerminalId,
    /// `terminal_incarnation`.
    TerminalIncarnation,
    /// `process_incarnation`, with its PID and start token.
    ProcessIncarnation,
    /// `stream_id` of an attach.
    StreamId,
    /// Event `seq`.
    Seq,
}

impl Field {
    /// Every field, in column order.
    pub const ALL: [Self; 8] = [
        Self::MachineId,
        Self::RuntimeLifetime,
        Self::ServerBootId,
        Self::TerminalId,
        Self::TerminalIncarnation,
        Self::ProcessIncarnation,
        Self::StreamId,
        Self::Seq,
    ];

    /// The wire field name.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::MachineId => "machine_id",
            Self::RuntimeLifetime => "runtime_lifetime",
            Self::ServerBootId => "server_boot_id",
            Self::TerminalId => "terminal_id",
            Self::TerminalIncarnation => "terminal_incarnation",
            Self::ProcessIncarnation => "process_incarnation",
            Self::StreamId => "stream_id",
            Self::Seq => "seq",
        }
    }
}

/// What happens to one identity under one cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Fate {
    /// The same value names the same thing afterwards.
    Kept,
    /// A new value is minted; the old one never names anything again.
    Replaced,
    /// The sequence continues above its last value in the same lifetime.
    Advances,
    /// A new sequence domain starts under the new lifetime; old cursors report loss.
    Rescoped,
}

impl Fate {
    /// The table token.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Kept => "kept",
            Self::Replaced => "replaced",
            Self::Advances => "advances",
            Self::Rescoped => "rescoped",
        }
    }
}

/// One row of the table: the fate of every [`Field`], in [`Field::ALL`] order.
const fn row(cause: Cause) -> [Fate; 8] {
    use Fate::{Advances as A, Kept as K, Replaced as R, Rescoped as S};
    match cause {
        Cause::ControlRestart | Cause::CoreCrash | Cause::Upgrade => [K, K, R, K, K, K, R, A],
        Cause::HolderLoss => [K, K, K, K, R, R, R, A],
        Cause::HostReboot => [K, R, R, K, R, R, R, S],
    }
}

/// The fate of `field` under `cause`.
#[must_use]
pub const fn fate(cause: Cause, field: Field) -> Fate {
    let [
        machine,
        lifetime,
        boot,
        terminal,
        terminal_incarnation,
        process_incarnation,
        stream,
        seq,
    ] = row(cause);
    match field {
        Field::MachineId => machine,
        Field::RuntimeLifetime => lifetime,
        Field::ServerBootId => boot,
        Field::TerminalId => terminal,
        Field::TerminalIncarnation => terminal_incarnation,
        Field::ProcessIncarnation => process_incarnation,
        Field::StreamId => stream,
        Field::Seq => seq,
    }
}

/// The table as published in `docs/identity-transitions.md`.
#[must_use]
pub fn markdown() -> String {
    let mut table = String::from("| cause |");
    let mut rule = String::from("|---|");
    for field in Field::ALL {
        table.push(' ');
        table.push_str(field.token());
        table.push_str(" |");
        rule.push_str("---|");
    }
    table.push('\n');
    table.push_str(&rule);
    table.push('\n');
    for cause in Cause::ALL {
        table.push_str("| ");
        table.push_str(cause.token());
        table.push_str(" |");
        for field in Field::ALL {
            table.push(' ');
            table.push_str(fate(cause, field).token());
            table.push_str(" |");
        }
        table.push('\n');
    }
    table
}
