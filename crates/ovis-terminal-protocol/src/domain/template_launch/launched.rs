//! Assembly of the published `terminal.run` result from a started launch.

use std::collections::BTreeMap;

use super::FirstMessageRecord;
use crate::schema::common::Geometry;
use crate::schema::ids::{MachineId, TerminalAddress, TerminalId, TerminalName};
use crate::schema::methods::terminal::{
    ChildPolicy, SpawnState, TerminalRunParams, TerminalRunResult,
};
use crate::schema::records::{
    ParentRef, ProcessRecord, StatusSnapshot, TerminalGeometry, TerminalOrigin, TerminalRecord,
    TerminalState,
};
use crate::schema::scalar::{ByteOffset, EventSeq, Incarnation, OpenEnum, Revision, TimestampMs};

/// Facts of a launched terminal, assembled into the published `terminal.run` result.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LaunchedTerminal {
    /// Stable terminal identity.
    pub terminal_id: TerminalId,
    /// Canonical address.
    pub address: TerminalAddress,
    /// Unique name, when given.
    pub name: Option<TerminalName>,
    /// Caller labels.
    pub labels: BTreeMap<String, String>,
    /// Owning machine.
    pub machine_id: MachineId,
    /// The program incarnation that was started.
    pub process: ProcessRecord,
    /// Parent binding by incarnation.
    pub parent: Option<ParentRef>,
    /// Policy toward the parent.
    pub child_policy: Option<ChildPolicy>,
    /// Whether the terminal is kept after its program dies.
    pub keep: bool,
    /// Initial grid.
    pub geometry: Geometry,
    /// Spawn outcome.
    pub spawn: SpawnState,
    /// Sequence of the creation event.
    pub created_seq: EventSeq,
}

impl LaunchedTerminal {
    /// A live terminal running `process`, confirmed spawned and created at `created_seq`;
    /// metadata comes from the launch parameters.
    #[must_use]
    pub fn new(
        (terminal_id, address, machine_id): (TerminalId, TerminalAddress, MachineId),
        (process, created_seq): (ProcessRecord, EventSeq),
        params: &TerminalRunParams,
    ) -> Self {
        Self {
            terminal_id,
            address,
            name: params.name.clone(),
            labels: params.labels.clone().unwrap_or_default(),
            machine_id,
            process,
            parent: params.parent.clone(),
            child_policy: params.child_policy,
            keep: params.keep.unwrap_or(false),
            geometry: params.geometry,
            spawn: SpawnState::Spawned,
            created_seq,
        }
    }

    /// The published result, carrying the first message queued under the launch.
    #[must_use]
    pub fn into_result(self, first_message: Option<FirstMessageRecord>) -> TerminalRunResult {
        let incarnation = self.process.incarnation;
        let policy = self.child_policy;
        TerminalRunResult {
            terminal: TerminalRecord {
                terminal_id: self.terminal_id,
                address: self.address,
                name: self.name,
                labels: self.labels,
                machine_id: self.machine_id,
                state: OpenEnum::Known(TerminalState::Live),
                origin: OpenEnum::Known(TerminalOrigin::Live),
                terminal_incarnation: Incarnation::new(1),
                process: Some(self.process),
                keep: self.keep,
                notify: policy.is_some_and(|policy| policy.notify),
                die_with_parent: policy
                    .map_or_else(|| self.parent.is_some(), |policy| policy.die_with_parent),
                parent: self.parent,
                geometry: TerminalGeometry {
                    size: self.geometry,
                    owner_lease: None,
                    revision: Revision::new(0),
                },
                output_offset: ByteOffset::new(0),
                revision: Revision::new(0),
                status: StatusSnapshot::unreported(incarnation),
                agent: None,
                finished: None,
                created_seq: self.created_seq,
                updated_seq: self.created_seq,
            },
            spawn: OpenEnum::Known(self.spawn),
            created_seq: self.created_seq,
            first_message,
        }
    }
}

/// One program incarnation as started: literal argv, working directory and start time.
#[must_use]
pub fn started_process(
    incarnation: Incarnation,
    (argv, cwd): (Vec<String>, Option<String>),
    started_ms: TimestampMs,
) -> ProcessRecord {
    ProcessRecord {
        incarnation,
        pid: None,
        start_token: None,
        argv,
        cwd,
        started_ms,
    }
}
