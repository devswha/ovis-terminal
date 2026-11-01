//! Assembling the published `server.status` and `server.metrics` results.

use super::{CensusState, ConsumerLoss, ConsumerRef, MethodLatency, RecoveryState};
use crate::schema::ids::{ClientId, RuntimeLifetimeId, ServerBootId};
use crate::schema::methods::server::{
    ConsumerQueue, RecoveryEvidence, ServerMetricsResult, ServerState, ServerStatusResult,
};
use crate::schema::scalar::{ByteCount, Count, DurationMs, EventSeq, OpenEnum, TimestampMs};

/// Identity a health record was collected under.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Collected {
    /// Holder-supervisor lifetime.
    pub runtime_lifetime: RuntimeLifetimeId,
    /// Control-server boot.
    pub server_boot_id: ServerBootId,
    /// Collection time.
    pub collected_ms: TimestampMs,
}

impl Collected {
    /// Collected at `collected_ms` under `(lifetime, boot)`.
    #[must_use]
    pub fn new(
        (runtime_lifetime, server_boot_id): (RuntimeLifetimeId, ServerBootId),
        collected_ms: TimestampMs,
    ) -> Self {
        Self {
            runtime_lifetime,
            server_boot_id,
            collected_ms,
        }
    }
}

/// Backlog facts of one consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Backlog {
    /// Unwritten bytes.
    pub queue_bytes: ByteCount,
    /// Age of the oldest unwritten item.
    pub event_lag_ms: DurationMs,
    /// Retained dropped ranges, oldest first.
    pub losses: Vec<ConsumerLoss>,
    /// Older ranges no longer retained.
    pub losses_omitted: Count,
}

impl Backlog {
    /// `(queued bytes, lag)` with `(retained losses, omitted count)`.
    #[must_use]
    pub fn new(
        (queue_bytes, event_lag_ms): (ByteCount, DurationMs),
        (losses, losses_omitted): (Vec<ConsumerLoss>, Count),
    ) -> Self {
        Self {
            queue_bytes,
            event_lag_ms,
            losses,
            losses_omitted,
        }
    }
}

impl ConsumerQueue {
    /// The row of `consumer`, owned by `client_id`.
    #[must_use]
    pub fn new(client_id: ClientId, consumer: ConsumerRef, backlog: Backlog) -> Self {
        let resynced = backlog
            .losses
            .iter()
            .filter(|loss| loss.resynced)
            .fold(0_u64, |total, loss| {
                total.saturating_add(loss.range.events())
            });
        Self {
            client_id,
            queue_bytes: backlog.queue_bytes,
            event_lag_ms: backlog.event_lag_ms,
            resynced_events: Count::new(resynced),
            consumer,
            losses: backlog.losses,
            losses_omitted: backlog.losses_omitted,
        }
    }
}

/// Everything `server.metrics` reports besides its identity.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Metrics {
    /// Active attach streams.
    pub streams: Count,
    /// Active event subscriptions.
    pub subscriptions: Count,
    /// Tracked consumers.
    pub consumers: Vec<ConsumerQueue>,
    /// Consumers refused tracking because the table was full.
    pub consumers_untracked: Count,
    /// Latest committed event sequence.
    pub latest_seq: EventSeq,
    /// Per-method latency.
    pub rpc_latency: Vec<MethodLatency>,
    /// Recovery evidence of this runtime lifetime.
    pub recovery: Vec<RecoveryEvidence>,
    /// Recovery summary.
    pub recovery_state: RecoveryState,
}

impl Metrics {
    /// `(streams, subscriptions, latest_seq)`, the consumer rows with the untracked
    /// count, and `(latency, recovery evidence, recovery summary)`.
    #[must_use]
    pub fn new(
        (streams, subscriptions, latest_seq): (Count, Count, EventSeq),
        (consumers, consumers_untracked): (Vec<ConsumerQueue>, Count),
        (rpc_latency, recovery, recovery_state): (
            Vec<MethodLatency>,
            Vec<RecoveryEvidence>,
            RecoveryState,
        ),
    ) -> Self {
        Self {
            streams,
            subscriptions,
            consumers,
            consumers_untracked,
            latest_seq,
            rpc_latency,
            recovery,
            recovery_state,
        }
    }
}

impl ServerMetricsResult {
    /// The metrics `metrics`, collected under `collected`.
    #[must_use]
    pub fn new(collected: Collected, metrics: Metrics) -> Self {
        Self {
            streams: metrics.streams,
            subscriptions: metrics.subscriptions,
            consumers: metrics.consumers,
            latest_seq: metrics.latest_seq,
            recovery: metrics.recovery,
            runtime_lifetime: collected.runtime_lifetime,
            server_boot_id: collected.server_boot_id,
            collected_ms: collected.collected_ms,
            consumers_untracked: metrics.consumers_untracked,
            rpc_latency: metrics.rpc_latency,
            recovery_state: metrics.recovery_state,
        }
    }
}

impl ServerStatusResult {
    /// A server answering for itself: running since `uptime_ms` with `census`
    /// `(holders, live terminals)` when one was published, both null otherwise.
    #[must_use]
    pub fn running(
        (runtime_lifetime, server_boot_id): (RuntimeLifetimeId, ServerBootId),
        (uptime_ms, census): (DurationMs, Option<(Count, Count)>),
        recovery_state: RecoveryState,
    ) -> Self {
        let state = if census.is_some() {
            CensusState::Reported
        } else {
            CensusState::NotReported
        };
        Self {
            state: OpenEnum::Known(ServerState::Running),
            runtime_lifetime: Some(runtime_lifetime),
            server_boot_id: Some(server_boot_id),
            uptime_ms: Some(uptime_ms),
            holder_count: census.map(|(holders, _)| holders),
            live_terminals: census.map(|(_, terminals)| terminals),
            census: OpenEnum::Known(state),
            recovery_state: Some(recovery_state),
        }
    }
}
