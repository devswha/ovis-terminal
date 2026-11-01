//! What a records read reports when no structured conversation exists.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ConversationSource;
use crate::schema::ids::AgentId;
use crate::schema::records::SessionRef;
use crate::schema::scalar::OpenEnum;

/// Why no structured conversation can be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum UnavailableReason {
    /// The terminal runs a plain program with no registered agent.
    NoAgent,
    /// The agent never reported typed records.
    NotReported,
}

/// What a conversation read consulted before finding nothing structured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Provenance {
    /// The source that was read.
    pub source: OpenEnum<ConversationSource>,
    /// Agent of the terminal's current program; null for a plain program.
    pub agent_id: Option<AgentId>,
    /// Session that agent declared, when it declared one.
    pub session: Option<SessionRef>,
}

/// Data of a `conversation_unavailable` refusal: the reason and what was consulted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Unavailable {
    /// Why nothing structured exists.
    pub reason: OpenEnum<UnavailableReason>,
    /// What the read consulted.
    pub provenance: Provenance,
}

impl Unavailable {
    /// `reason`, found reading `source` for `agent_id` bound to `session`.
    #[must_use]
    pub fn new(
        reason: UnavailableReason,
        source: ConversationSource,
        (agent_id, session): (Option<AgentId>, Option<SessionRef>),
    ) -> Self {
        Self {
            reason: OpenEnum::Known(reason),
            provenance: Provenance {
                source: OpenEnum::Known(source),
                agent_id,
                session,
            },
        }
    }
}
