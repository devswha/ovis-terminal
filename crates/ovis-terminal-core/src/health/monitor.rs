use std::sync::{PoisonError, RwLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use ovis_terminal_protocol::domain::health::{Collected, Metrics};
use ovis_terminal_protocol::schema::ids::{RuntimeLifetimeId, ServerBootId};
use ovis_terminal_protocol::schema::methods::server::{ServerMetricsResult, ServerStatusResult};
use ovis_terminal_protocol::schema::scalar::{DurationMs, EventSeq, TimestampMs};
use ovis_terminal_store::RecoveryEvidence as Quarantined;

use super::{Consumers, HealthError, Latencies, RecoveryLedger};
use crate::registry::RuntimeIdentity;

/// Holders and live terminals, as published by the terminal service.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Census {
    /// Live holder processes.
    pub holders: u32,
    /// Live terminals.
    pub live_terminals: u32,
}

impl Census {
    /// `holders` live holder processes serving `live_terminals` terminals.
    #[must_use]
    pub const fn new(holders: u32, live_terminals: u32) -> Self {
        Self {
            holders,
            live_terminals,
        }
    }
}

/// What a read observed in the store's committed snapshot.
#[derive(Debug)]
#[non_exhaustive]
pub struct Observed {
    /// Latest committed event sequence.
    pub latest_seq: EventSeq,
    /// Quarantined store material, durable in the database.
    pub quarantined: Vec<Quarantined>,
}

impl Observed {
    /// The snapshot's latest sequence and quarantined store material.
    #[must_use]
    pub const fn new(latest_seq: EventSeq, quarantined: Vec<Quarantined>) -> Self {
        Self {
            latest_seq,
            quarantined,
        }
    }
}

/// The health of one control-server boot.
#[derive(Debug)]
pub struct Health {
    lifetime: RuntimeLifetimeId,
    boot: ServerBootId,
    started: Instant,
    consumers: Consumers,
    latencies: Latencies,
    recovery: RecoveryLedger,
    census: RwLock<Census>,
}

fn wall_clock() -> Result<TimestampMs, HealthError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .map(TimestampMs::new)
        .ok_or(HealthError::Clock)
}

impl Health {
    /// Health of `runtime`'s boot, started now, with `recovery`'s facts.
    #[must_use]
    pub fn new(runtime: RuntimeIdentity, recovery: RecoveryLedger) -> Self {
        Self {
            lifetime: RuntimeLifetimeId::new(runtime.runtime_id().to_string()),
            boot: ServerBootId::new(runtime.server_boot_id().to_string()),
            started: Instant::now(),
            consumers: Consumers::new(),
            latencies: Latencies::new(),
            recovery,
            census: RwLock::new(Census::default()),
        }
    }

    /// The consumer table producers report into.
    #[must_use]
    pub const fn consumers(&self) -> &Consumers {
        &self.consumers
    }

    /// The per-method latency table.
    #[must_use]
    pub const fn latencies(&self) -> &Latencies {
        &self.latencies
    }

    /// The recovery facts.
    #[must_use]
    pub const fn recovery(&self) -> &RecoveryLedger {
        &self.recovery
    }

    /// Replaces the published census.
    pub fn publish_census(&self, census: Census) {
        *self.census.write().unwrap_or_else(PoisonError::into_inner) = census;
    }

    /// This server's own status; a server that answers is running.
    #[must_use]
    pub fn status(&self, quarantined: &[Quarantined]) -> ServerStatusResult {
        let census = *self.census.read().unwrap_or_else(PoisonError::into_inner);
        let uptime = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        ServerStatusResult::running(
            (self.lifetime.clone(), self.boot.clone()),
            (
                DurationMs::new(uptime),
                census.holders,
                census.live_terminals,
            ),
            self.recovery.state(quarantined),
        )
    }

    /// Every published metric, stamped with this boot and the collection time.
    ///
    /// # Errors
    ///
    /// [`HealthError::Clock`] when the wall clock reads before the Unix epoch.
    pub fn metrics(&self, observed: &Observed) -> Result<ServerMetricsResult, HealthError> {
        let collected_ms = wall_clock()?;
        let rows = self.consumers.rows(collected_ms);
        let metrics = Metrics::new(
            (rows.streams, rows.subscriptions, observed.latest_seq),
            (rows.consumers, rows.untracked),
            (
                self.latencies.rows(),
                self.recovery.evidence(),
                self.recovery.state(&observed.quarantined),
            ),
        );
        let collected = Collected::new((self.lifetime.clone(), self.boot.clone()), collected_ms);
        Ok(ServerMetricsResult::new(collected, metrics))
    }
}
