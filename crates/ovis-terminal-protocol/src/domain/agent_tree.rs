//! Parent/child links bound to parent program incarnations, declared child policy and
//! durable parent-death facts (plan todo 41).
//!
//! A link names its parent by terminal and program incarnation, never by a reusable
//! address, and keeps the parent's session identity as it stood when the child was
//! linked; child status never rewrites it. A parent's death is a durable fact carrying
//! one acknowledgement per machine that holds its children. News about a child is a
//! factual record its parent acknowledges, never a toast, banner or sound.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::schema::ids::{AgentId, MachineId, TerminalId};
use crate::schema::methods::agent::{AgentChildrenResult, ChildRecord};
use crate::schema::methods::terminal::ChildPolicy;
use crate::schema::records::{ParentRef, SessionRef, StatusSnapshot};
use crate::schema::scalar::{EventSeq, Incarnation, OpenEnum, Revision, TurnNumber};

/// One program incarnation of a child terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ChildRef {
    /// Child terminal.
    pub terminal_id: TerminalId,
    /// Child program incarnation.
    pub process_incarnation: Incarnation,
}

impl ChildRef {
    /// Program `process_incarnation` of `terminal_id`.
    #[must_use]
    pub const fn new(terminal_id: TerminalId, process_incarnation: Incarnation) -> Self {
        Self {
            terminal_id,
            process_incarnation,
        }
    }
}

impl ParentRef {
    /// Program `incarnation` of `terminal_id`.
    #[must_use]
    pub const fn new(terminal_id: TerminalId, incarnation: Incarnation) -> Self {
        Self {
            terminal_id,
            incarnation,
        }
    }
}

impl ChildPolicy {
    /// Declared policy: `notify` the parent of news, end with the parent when `die_with_parent`.
    #[must_use]
    pub const fn new(notify: bool, die_with_parent: bool) -> Self {
        Self {
            notify,
            die_with_parent,
        }
    }
}

/// Lifecycle of a linked child program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChildState {
    /// The child program runs.
    Live,
    /// Its parent finished and the child opted to die with it: an end was ordered, but
    /// no exit has been observed yet.
    Ending,
    /// The child program's exit was observed.
    Ended,
}

/// Why a child program ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EndReason {
    /// It exited while its parent's end order was not pending.
    Exited,
    /// It exited after an end ordered because its parent incarnation finished.
    WithParent,
}

/// The observed end of a child program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ChildEnd {
    /// Why it ended.
    pub reason: OpenEnum<EndReason>,
    /// Sequence of the event that recorded the exit.
    pub seq: EventSeq,
}

/// The parent's own agent identity as it stood when a child was linked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ParentIdentity {
    /// Registered agent of the parent program, if any.
    pub agent_id: Option<AgentId>,
    /// Provider session of the parent program, if any.
    pub session: Option<SessionRef>,
    /// The parent's turn number.
    pub turn: TurnNumber,
}

impl ParentIdentity {
    /// The identity `status` declares for the parent program.
    #[must_use]
    pub fn of(status: &StatusSnapshot) -> Self {
        Self {
            agent_id: status.agent_id.clone(),
            session: status.session.clone(),
            turn: status.turn,
        }
    }
}

/// A child program linked to one parent program incarnation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ChildLink {
    /// The child program.
    pub child: ChildRef,
    /// The parent program, by incarnation.
    pub parent: ParentRef,
    /// Machine the parent runs on.
    pub parent_machine: MachineId,
    /// Declared policy toward the parent.
    pub policy: ChildPolicy,
    /// The parent's identity at link time, when the parent runs on this machine; child
    /// status never changes it.
    pub parent_identity: Option<ParentIdentity>,
    /// Sequence of the unit that linked the child.
    pub linked_seq: EventSeq,
    /// Lifecycle of the child program.
    pub state: OpenEnum<ChildState>,
    /// The observed end, once the child's exit is known.
    pub ended: Option<ChildEnd>,
}

impl ChildLink {
    /// A live `child` of `parent` on `parent_machine`, linked at `linked_seq`.
    #[must_use]
    pub fn linked(
        (child, parent, parent_machine): (ChildRef, ParentRef, MachineId),
        (policy, parent_identity): (ChildPolicy, Option<ParentIdentity>),
        linked_seq: EventSeq,
    ) -> Self {
        Self {
            child,
            parent,
            parent_machine,
            policy,
            parent_identity,
            linked_seq,
            state: OpenEnum::Known(ChildState::Live),
            ended: None,
        }
    }

    /// Orders the end of a live child that opted to die with its parent; returns whether
    /// an end was ordered. Independent and already ending or ended children are unchanged.
    pub fn order_end(&mut self) -> bool {
        let ordered =
            self.policy.die_with_parent && self.state == OpenEnum::Known(ChildState::Live);
        if ordered {
            self.state = OpenEnum::Known(ChildState::Ending);
        }
        ordered
    }

    /// Records the observed exit at `seq`; returns the reason, or `None` when the exit
    /// was already recorded.
    pub fn exited(&mut self, seq: EventSeq) -> Option<EndReason> {
        let reason = match self.state {
            OpenEnum::Known(ChildState::Live) => EndReason::Exited,
            OpenEnum::Known(ChildState::Ending) => EndReason::WithParent,
            _ => return None,
        };
        self.state = OpenEnum::Known(ChildState::Ended);
        self.ended = Some(ChildEnd {
            reason: OpenEnum::Known(reason),
            seq,
        });
        Some(reason)
    }

    /// Whether the child program still runs or has not yet been seen to exit.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.state != OpenEnum::Known(ChildState::Ended)
    }
}

/// What a notice tells the parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum NoticeKind {
    /// The child exited on its own.
    Exited,
    /// The child ended because its parent incarnation finished.
    EndedWithParent,
}

impl From<EndReason> for NoticeKind {
    fn from(reason: EndReason) -> Self {
        match reason {
            EndReason::Exited => Self::Exited,
            EndReason::WithParent => Self::EndedWithParent,
        }
    }
}

/// A factual record of child news for a parent that asked to be notified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ChildNotice {
    /// Sequence of the event that recorded the news; the notice's identity.
    pub notice_seq: EventSeq,
    /// The child the news is about.
    pub child: ChildRef,
    /// What happened.
    pub kind: OpenEnum<NoticeKind>,
    /// Sequence of the acknowledgement, once the parent acknowledged it.
    pub acknowledged_seq: Option<EventSeq>,
}

impl ChildNotice {
    /// Unacknowledged `kind` news about `child`, recorded at `notice_seq`.
    #[must_use]
    pub const fn new(child: ChildRef, kind: NoticeKind, notice_seq: EventSeq) -> Self {
        Self {
            notice_seq,
            child,
            kind: OpenEnum::Known(kind),
            acknowledged_seq: None,
        }
    }
}

/// The parent's acknowledgement of every notice up to a sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct NoticeAck {
    /// The acknowledging parent program.
    pub parent: ParentRef,
    /// Highest acknowledged notice sequence.
    pub through: EventSeq,
    /// Notices this acknowledgement covered for the first time.
    pub acknowledged: Vec<EventSeq>,
}

impl NoticeAck {
    /// `parent` acknowledged every notice through `through`, `acknowledged` for the first time.
    #[must_use]
    pub const fn new(parent: ParentRef, through: EventSeq, acknowledged: Vec<EventSeq>) -> Self {
        Self {
            parent,
            through,
            acknowledged,
        }
    }
}

impl ChildRecord {
    /// `child` with its declared `status` and `unseen_news` unacknowledged notices.
    #[must_use]
    pub fn new(child: ChildRef, status: StatusSnapshot, unseen_news: u32) -> Self {
        Self {
            terminal_id: child.terminal_id,
            process_incarnation: child.process_incarnation,
            status,
            unseen_news,
        }
    }
}

impl AgentChildrenResult {
    /// The listed `children` as of `revision`.
    #[must_use]
    pub const fn new(children: Vec<ChildRecord>, revision: Revision) -> Self {
        Self { children, revision }
    }
}

mod orphan;

pub use orphan::{OrphanAck, OrphanNotice, ParentDeath, RemoteDelivery};

#[cfg(test)]
mod tests;
