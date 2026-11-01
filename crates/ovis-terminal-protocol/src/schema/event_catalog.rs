//! Every event type and the schema of its data.

use super::events::{
    EndedData, ForegroundData, LeaseEventData, OutputRevisionData, RemovedData, RestartingData,
    SpawnedData, StreamClosedData,
};
use super::method::{EventSpec, event};
use super::methods::agent::AgentNoteResult;
use super::methods::doc::DocEntry;
use super::methods::layout::{LayoutTree, Selection};
use super::methods::machine::MachineRecord;
use super::methods::plugin::PluginRunRecord;
use super::methods::server::{OpRecord, RecoveryEvidence};
use super::methods::status::{PromptRecord, TurnRecord};
use super::methods::update::UpdateRecord;
use super::methods::worktree::WorktreeRecord;
use super::records::{AgentBinding, StatusSnapshot, TerminalGeometry, TerminalRecord};
use crate::domain::conversation_records::RecordsAppended;
use crate::domain::graphics::{GraphicsDroppedData, GraphicsEvictedData};
use crate::domain::session_binding::SessionBindingRecord;

/// Every event type, in publication order of the schema.
pub(super) const EVENTS: &[EventSpec] = &[
    event!(
        "op.accepted",
        "Operation acceptance was persisted.",
        OpRecord
    ),
    event!(
        "op.running",
        "Operation intent was persisted before effects.",
        OpRecord
    ),
    event!(
        "op.cancelled",
        "Operation was cancelled before effects.",
        OpRecord
    ),
    event!(
        "op.outcome_unknown",
        "Operation effects cannot yet be proven.",
        OpRecord
    ),
    event!("terminal.created", "A launch committed.", TerminalRecord),
    event!(
        "terminal.spawned",
        "The holder confirmed a spawn.",
        SpawnedData
    ),
    event!(
        "terminal.lost",
        "The program died and the terminal is kept.",
        EndedData
    ),
    event!(
        "terminal.restarting",
        "Recovery scheduled a relaunch.",
        RestartingData
    ),
    event!(
        "terminal.restarted",
        "Recovery relaunched the program.",
        TerminalRecord
    ),
    event!(
        "terminal.finished",
        "The program ended after its final bytes were retained.",
        EndedData
    ),
    event!("terminal.removed", "A terminal was removed.", RemovedData),
    event!(
        "terminal.updated",
        "Terminal metadata changed.",
        TerminalRecord
    ),
    event!("terminal.size", "Geometry was committed.", TerminalGeometry),
    event!(
        "terminal.foreground",
        "The foreground process changed.",
        ForegroundData
    ),
    event!(
        "output.revision",
        "The screen advanced.",
        OutputRevisionData
    ),
    event!(
        "status.changed",
        "A status report, clear or exit invalidation was accepted.",
        StatusSnapshot
    ),
    event!("turn.ended", "The root status left working.", TurnRecord),
    event!("prompt.opened", "A prompt opened.", PromptRecord),
    event!("prompt.updated", "A prompt changed version.", PromptRecord),
    event!("prompt.closed", "A prompt closed.", PromptRecord),
    event!("agent.registered", "An agent registered.", AgentBinding),
    event!("agent.updated", "An agent binding changed.", AgentBinding),
    event!(
        "agent.session_bound",
        "An owning process bound, resumed, branched or replaced its session.",
        SessionBindingRecord
    ),
    event!(
        "agent.records_appended",
        "An owning process appended typed conversation records.",
        RecordsAppended
    ),
    event!(
        "agent.released",
        "An agent binding was released.",
        AgentBinding
    ),
    event!(
        "agent.note",
        "A note was delivered or queued.",
        AgentNoteResult
    ),
    event!("lease.acquired", "A lease was acquired.", LeaseEventData),
    event!("lease.revoked", "A lease was revoked.", LeaseEventData),
    event!(
        "lease.geometry",
        "Geometry ownership moved.",
        LeaseEventData
    ),
    event!("stream.closed", "A stream closed.", StreamClosedData),
    event!("op.committed", "A long operation committed.", OpRecord),
    event!("op.failed", "A long operation failed.", OpRecord),
    event!("doc.changed", "A document version changed.", DocEntry),
    event!(
        "machine.changed",
        "A machine's connection state changed.",
        MachineRecord
    ),
    event!(
        "server.recovery",
        "A boot or restart boundary recorded recovery evidence.",
        RecoveryEvidence
    ),
    event!("layout.changed", "The layout changed.", LayoutTree),
    event!(
        "selection.changed",
        "One client's selection changed.",
        Selection
    ),
    event!("worktree.changed", "A checkout changed.", WorktreeRecord),
    event!("plugin.run", "A plugin run changed state.", PluginRunRecord),
    event!("update.progress", "An update changed state.", UpdateRecord),
    event!(
        "graphics.dropped",
        "A graphics sequence exceeded its cap and was dropped whole.",
        GraphicsDroppedData
    ),
    event!(
        "graphics.evicted",
        "A terminal's graphics store was full and its oldest sequences were evicted.",
        GraphicsEvictedData
    ),
];
