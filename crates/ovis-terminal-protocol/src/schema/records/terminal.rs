//! Terminal and program incarnation records.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{AgentBinding, StatusSnapshot};
use crate::schema::common::Geometry;
use crate::schema::ids::{
    LeaseId, MachineId, ProcessPid, StartToken, TerminalAddress, TerminalId, TerminalName,
};
use crate::schema::scalar::{ByteOffset, EventSeq, Incarnation, OpenEnum, Revision, TimestampMs};

/// Lifecycle state of a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TerminalState {
    /// A program is running.
    Live,
    /// The program ended and the terminal is not kept.
    Finished,
    /// The program died and the terminal is kept for recovery.
    Lost,
    /// Recovery is relaunching the program.
    Restarting,
}

/// How the current terminal record came to exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TerminalOrigin {
    /// Created and running in this runtime lifetime.
    Live,
    /// Metadata restored after reboot; no program runs.
    RestoredMetadata,
    /// Relaunched after recovery with a new program incarnation.
    Relaunched,
}

/// Why a program finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FinishReason {
    /// The program exited on its own.
    Exited,
    /// A kill request ended the program.
    Killed,
    /// A signal ended the program.
    Signaled,
    /// The server stopped together with its programs.
    ServerStop,
    /// The host shut down.
    HostShutdown,
    /// The holder process was lost.
    HolderLost,
    /// The program could not be spawned.
    SpawnFailed,
}

/// One spawned program incarnation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ProcessRecord {
    /// Program incarnation; bumps on every spawn.
    pub incarnation: Incarnation,
    /// Operating-system process identifier, when known.
    pub pid: Option<ProcessPid>,
    /// Token proving which program the identifier names.
    pub start_token: Option<StartToken>,
    /// Argument vector the program was started with.
    pub argv: Vec<String>,
    /// Working directory at spawn.
    pub cwd: Option<String>,
    /// Spawn time.
    pub started_ms: TimestampMs,
}

/// Facts about a finished program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct FinishedRecord {
    /// Why the program finished.
    pub reason: OpenEnum<FinishReason>,
    /// Exit code, when the program exited.
    pub exit_code: Option<i32>,
    /// Signal number, when a signal ended the program.
    pub signal: Option<i32>,
    /// Finish time.
    pub finished_ms: TimestampMs,
    /// Raw output offset after the final bytes were retained.
    pub final_output_offset: ByteOffset,
}

/// Parent terminal bound by incarnation, never by a reusable address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ParentRef {
    /// Parent terminal identity.
    pub terminal_id: TerminalId,
    /// Parent program incarnation.
    pub incarnation: Incarnation,
}

/// Committed geometry and its owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TerminalGeometry {
    /// Current grid size.
    pub size: Geometry,
    /// Lease that owns geometry, when any.
    pub owner_lease: Option<LeaseId>,
    /// Revision at which this geometry was committed.
    pub revision: Revision,
}

/// Full terminal record shared by reads, snapshots and events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TerminalRecord {
    /// Stable terminal identity.
    pub terminal_id: TerminalId,
    /// Canonical address.
    pub address: TerminalAddress,
    /// Optional unique name.
    pub name: Option<TerminalName>,
    /// Caller-defined labels.
    pub labels: BTreeMap<String, String>,
    /// Owning machine.
    pub machine_id: MachineId,
    /// Lifecycle state.
    pub state: OpenEnum<TerminalState>,
    /// How this record came to exist.
    pub origin: OpenEnum<TerminalOrigin>,
    /// Terminal incarnation; bumps on keep and recover.
    pub terminal_incarnation: Incarnation,
    /// Current or last program incarnation.
    pub process: Option<ProcessRecord>,
    /// Whether the terminal is kept after its program dies.
    pub keep: bool,
    /// Parent binding, when launched as a child.
    pub parent: Option<ParentRef>,
    /// Whether child news becomes notes to the parent.
    pub notify: bool,
    /// Whether the parent incarnation's end kills this terminal.
    pub die_with_parent: bool,
    /// Committed geometry.
    pub geometry: TerminalGeometry,
    /// Cumulative raw output offset.
    pub output_offset: ByteOffset,
    /// Record revision.
    pub revision: Revision,
    /// Declared status.
    pub status: StatusSnapshot,
    /// Registered agent, or null for a plain program.
    pub agent: Option<AgentBinding>,
    /// Finish facts, when finished.
    pub finished: Option<FinishedRecord>,
    /// Sequence of the creating event.
    pub created_seq: EventSeq,
    /// Sequence of the latest change.
    pub updated_seq: EventSeq,
}
