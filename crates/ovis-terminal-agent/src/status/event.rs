//! Agent status events in the coordinator's sequence domain.

use ovis_terminal_core::events::EventData;
use ovis_terminal_core::ops::OperationEvent;
use ovis_terminal_protocol::domain::session_binding::SessionBindingRecord;
use ovis_terminal_protocol::schema::methods::status::TurnRecord;
use ovis_terminal_protocol::schema::records::{AgentBinding, StatusSnapshot};
use ovis_terminal_protocol::schema::scalar::EventSeq;
use serde::{Deserialize, Serialize};

/// Status, turn and agent facts committed with their state rows and receipts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AgentStatusEvent {
    /// A report, clear or exit invalidation was accepted.
    StatusChanged(Box<StatusSnapshot>),
    /// The root status left working.
    TurnEnded(Box<TurnRecord>),
    /// An agent registered.
    AgentRegistered(Box<AgentBinding>),
    /// An owning process bound, resumed, branched or replaced its session.
    SessionBound(Box<SessionBindingRecord>),
    /// Retains the released operation event's unwrapped representation.
    #[serde(untagged)]
    Operation(Box<OperationEvent>),
}

impl From<OperationEvent> for AgentStatusEvent {
    fn from(event: OperationEvent) -> Self {
        Self::Operation(Box::new(event))
    }
}

impl EventData for AgentStatusEvent {
    fn event_type(&self) -> &'static str {
        match self {
            Self::StatusChanged(_) => "status.changed",
            Self::TurnEnded(_) => "turn.ended",
            Self::AgentRegistered(_) => "agent.registered",
            Self::SessionBound(_) => "agent.session_bound",
            Self::Operation(event) => event.event_type(),
        }
    }

    fn at_sequence(&mut self, seq: EventSeq) {
        match self {
            Self::Operation(event) => event.at_sequence(seq),
            Self::StatusChanged(_)
            | Self::TurnEnded(_)
            | Self::AgentRegistered(_)
            | Self::SessionBound(_) => {}
        }
    }
}
