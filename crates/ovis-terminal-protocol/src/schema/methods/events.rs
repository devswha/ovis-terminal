//! `events.*`: independent subscriptions with snapshots, replay and explicit loss.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::schema::common::{EventCursor, MachineRef, Target};
use crate::schema::ids::{SubscriptionId, TerminalId};
use crate::schema::method::{MethodSpec, method};
use crate::schema::records::{AgentState, TerminalRecord};
use crate::schema::scalar::{ByteCount, DurationMs, EventSeq, OpenEnum, TurnNumber};

/// Which committed events a subscription delivers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EventFilter {
    /// Only events about these terminals.
    pub terminals: Option<Vec<TerminalId>>,
    /// Only events about children of these terminals.
    pub children_of: Option<Vec<TerminalId>>,
    /// Only events about terminals carrying all of these labels.
    pub labels: Option<BTreeMap<String, String>>,
    /// Only these event types.
    pub types: Option<Vec<String>>,
    /// Only events from these machines.
    pub machines: Option<Vec<MachineRef>>,
}

/// Parameters of `events.subscribe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EventsSubscribeParams {
    /// Filter.
    pub filter: EventFilter,
    /// Resume after this cursor; absent starts from a snapshot.
    pub cursor: Option<EventCursor>,
    /// Include a snapshot taken atomically with filter installation.
    pub snapshot: bool,
}

/// Why replay from a cursor was impossible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MissedReason {
    /// The cursor is older than the replay floor.
    CursorExpired,
    /// The cursor belongs to another runtime lifetime.
    LifetimeMismatch,
}

/// Result of `events.subscribe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct EventsSubscribeResult {
    /// Subscription identity.
    pub subscription_id: SubscriptionId,
    /// Snapshot terminals, when requested or when replay was impossible.
    pub snapshot: Option<Vec<TerminalRecord>>,
    /// Sequence the snapshot reflects; later events have higher sequences.
    pub snapshot_seq: EventSeq,
    /// Oldest sequence available for replay.
    pub replay_floor: EventSeq,
    /// Why replay was impossible, when it was.
    pub missed: Option<OpenEnum<MissedReason>>,
    /// Queue bound in events.
    pub queue_events: u32,
    /// Queue bound in bytes.
    pub queue_bytes: ByteCount,
    /// Idle heartbeat interval.
    pub heartbeat_ms: DurationMs,
}

/// Parameters of `events.update`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EventsUpdateParams {
    /// Subscription.
    pub subscription_id: SubscriptionId,
    /// Replacement filter.
    pub filter: EventFilter,
}

/// Result of `events.update`; delivery continues across the change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct EventsUpdateResult {
    /// Subscription.
    pub subscription_id: SubscriptionId,
    /// First sequence evaluated against the new filter.
    pub effective_seq: EventSeq,
}

/// Parameters of `events.unsubscribe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EventsUnsubscribeParams {
    /// Subscription.
    pub subscription_id: SubscriptionId,
}

/// Parameters of `events.ack`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EventsAckParams {
    /// Subscription.
    pub subscription_id: SubscriptionId,
    /// Highest sequence the client processed.
    pub seq: EventSeq,
}

/// Result of `events.unsubscribe` and `events.ack`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct EventsCursorResult {
    /// Highest sequence acknowledged or closed at.
    pub seq: EventSeq,
    /// Highest sequence examined for this subscription.
    pub scan_seq: EventSeq,
}

/// Parameters of `events.wait`; the condition is evaluated before subscribing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EventsWaitParams {
    /// Terminal to watch.
    pub target: Target,
    /// Wait for one of these declared states.
    pub states: Option<Vec<OpenEnum<AgentState>>>,
    /// Wait for a turn after this one.
    pub turn_after: Option<TurnNumber>,
    /// Wait for the program to exit.
    pub exit: Option<bool>,
    /// Evaluate events after this cursor.
    pub cursor: Option<EventCursor>,
    /// Wait bound.
    pub timeout_ms: DurationMs,
}

/// How an event wait ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EventWaitOutcome {
    /// A matching event was committed.
    Matched,
    /// The condition already held at evaluation.
    AlreadySatisfied,
    /// The bound passed.
    Timeout,
    /// The terminal was lost.
    Lost,
    /// The program exited.
    Exited,
}

/// Result of `events.wait`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct EventsWaitResult {
    /// How the wait ended.
    pub outcome: OpenEnum<EventWaitOutcome>,
    /// Sequence of the deciding event.
    pub seq: EventSeq,
}

/// Methods of this namespace.
pub(crate) const METHODS: &[MethodSpec] = &[
    method! {"events.subscribe", Control, Untargeted, EventsSubscribeParams => EventsSubscribeResult,
    "Install a filter atomically with a snapshot or replay from a cursor.", [CursorExpired, LifetimeMismatch, FilterInvalid, SubscriptionLimit]},
    method! {"events.update", Control, Lookup, EventsUpdateParams => EventsUpdateResult,
    "Replace a subscription's filter without interrupting delivery.", [FilterInvalid]},
    method! {"events.unsubscribe", Control, Lookup, EventsUnsubscribeParams => EventsCursorResult,
    "Close one subscription; others are unaffected.", []},
    method! {"events.ack", Control, Lookup, EventsAckParams => EventsCursorResult,
    "Acknowledge processed events.", [SeqAhead]},
    method! {"events.wait", Query, Terminal, EventsWaitParams => EventsWaitResult,
    "Evaluate a condition, then wait for it without polling.", [CursorExpired, FilterInvalid]},
];
