//! Read-only health and per-consumer metrics records (plan todo 59).
//!
//! `server.status` and `server.metrics` report what the runtime already published: no
//! read starts a server, acknowledges a recovery failure or resets a counter. Every
//! record names the runtime lifetime and boot it was collected from, so a reader can tell
//! a stale answer from a current one, and every consumer row names the stream or
//! subscription and the terminal it reads.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::errors::OpId;
use crate::schema::ids::{MethodName, StreamId, SubscriptionId, TerminalId};
use crate::schema::scalar::{ByteOffset, Count, DurationMs, EventSeq, OpenEnum};

mod results;
#[cfg(test)]
mod tests;

pub use results::{Backlog, Collected, Metrics};

/// The outbound path one consumer reads.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum ConsumerRef {
    /// An attach output stream of one terminal.
    Stream {
        /// The stream.
        stream_id: StreamId,
        /// The terminal it carries.
        terminal_id: TerminalId,
    },
    /// An event subscription.
    Subscription {
        /// The subscription.
        subscription_id: SubscriptionId,
    },
}

/// An inclusive range a consumer did not receive in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "unit", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum LossRange {
    /// Event sequence numbers `first_seq..=last_seq`.
    Events {
        /// First lost event.
        first_seq: EventSeq,
        /// Last lost event.
        last_seq: EventSeq,
    },
    /// Stream bytes `first_offset..=last_offset`.
    Bytes {
        /// First lost byte offset.
        first_offset: ByteOffset,
        /// Last lost byte offset.
        last_offset: ByteOffset,
    },
}

impl LossRange {
    /// The number of events the range covers; zero for a byte range.
    #[must_use]
    pub const fn events(&self) -> u64 {
        match self {
            Self::Events {
                first_seq,
                last_seq,
            } => last_seq
                .get()
                .saturating_sub(first_seq.get())
                .saturating_add(1),
            Self::Bytes { .. } => 0,
        }
    }
}

/// One dropped range and whether the consumer was resynchronized past it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ConsumerLoss {
    /// What was dropped.
    pub range: LossRange,
    /// A snapshot or resync replaced the dropped range.
    pub resynced: bool,
}

impl ConsumerLoss {
    /// `range`, dropped and not yet resynchronized.
    #[must_use]
    pub const fn dropped(range: LossRange) -> Self {
        Self {
            range,
            resynced: false,
        }
    }

    /// `range`, already replaced by a snapshot or resync.
    #[must_use]
    pub const fn replaced(range: LossRange) -> Self {
        Self {
            range,
            resynced: true,
        }
    }
}

/// Latency of one served method since this boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MethodLatency {
    /// The method.
    pub method: MethodName,
    /// Settled calls.
    pub calls: Count,
    /// Calls settled with an error.
    pub failures: Count,
    /// Latency of the most recent call.
    pub last_ms: DurationMs,
    /// Largest latency seen.
    pub max_ms: DurationMs,
    /// Operation of the slowest call, when that call named one.
    pub slowest_op: Option<OpId>,
}

impl MethodLatency {
    /// A row for `method` with its counts and latencies.
    #[must_use]
    pub fn new(
        method: MethodName,
        (calls, failures): (Count, Count),
        (last_ms, max_ms, slowest_op): (DurationMs, DurationMs, Option<OpId>),
    ) -> Self {
        Self {
            method,
            calls,
            failures,
            last_ms,
            max_ms,
            slowest_op,
        }
    }
}

/// Whether a status answer carries a holder and terminal census.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CensusState {
    /// The terminal service published `holder_count` and `live_terminals`.
    Reported,
    /// Nothing published a census; both counts are null rather than zero.
    NotReported,
}

/// Schema of a census count that is always present: a decimal string, or null when no
/// census was reported.
pub(crate) fn nullable_count(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    generator.subschema_for::<Option<Count>>()
}

/// The census state of an answer that carries none.
#[must_use]
pub const fn census_not_reported() -> OpenEnum<CensusState> {
    OpenEnum::Known(CensusState::NotReported)
}

/// Summary of what recovery found; reading it never clears it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RecoveryPhase {
    /// No recovery and no incident was ever recorded.
    Clean,
    /// This lifetime recovered or recorded incidents, and every terminal is settled.
    Recovered,
    /// Damaged material was recorded or a terminal's survival is still unknown.
    Degraded,
}

/// Recovery state of the runtime, durable across reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RecoveryState {
    /// Overall phase.
    pub phase: OpenEnum<RecoveryPhase>,
    /// Damaged database, journal or checkpoint material was recorded.
    pub corrupted: bool,
    /// Recorded incidents and quarantined store material.
    pub incidents: Count,
    /// Terminals whose program survival is still unknown.
    pub unsettled: Count,
}

impl RecoveryState {
    /// The state these facts imply: damage or unsettled survival is degraded, any other
    /// recorded fact is recovered, and nothing at all is clean.
    #[must_use]
    pub const fn derive(
        corrupted: bool,
        (incidents, unsettled): (u64, u64),
        recovered: bool,
    ) -> Self {
        let phase = if corrupted || unsettled > 0 {
            RecoveryPhase::Degraded
        } else if recovered || incidents > 0 {
            RecoveryPhase::Recovered
        } else {
            RecoveryPhase::Clean
        };
        Self {
            phase: OpenEnum::Known(phase),
            corrupted,
            incidents: Count::new(incidents),
            unsettled: Count::new(unsettled),
        }
    }
}
