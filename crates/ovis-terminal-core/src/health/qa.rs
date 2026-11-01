use std::path::PathBuf;
use std::time::Duration;

use ovis_terminal_protocol::domain::health::{CensusState, ConsumerRef, LossRange};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::{ClientId, StreamId, SubscriptionId, TerminalId};
use ovis_terminal_protocol::schema::methods::server::{ConsumerQueue, ServerMetricsResult};
use ovis_terminal_protocol::schema::scalar::{ByteOffset, Count, EventSeq, OpenEnum, TimestampMs};

use super::{
    Census, Health, HealthError, Latencies, MAX_CONSUMERS, MAX_LOSSES, Observed, RecoveryLedger,
    Settled,
};
use crate::recovery::Incidents;
use crate::registry::{
    HostBootId, HostIdentity, MachineId, RuntimeId, RuntimeIdentity, ServerBootId,
};

mod recovery;

pub(super) struct StateDir(pub(super) PathBuf);

impl StateDir {
    pub(super) fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("ovis-health-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).expect("state dir");
        Self(dir)
    }
}

impl Drop for StateDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove state dir");
    }
}

pub(super) fn runtime() -> RuntimeIdentity {
    let machine: MachineId = "machine:0191f1a2-0000-7000-8000-000000000059"
        .parse()
        .expect("machine");
    RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1).expect("host boot")),
        RuntimeId::try_from(59).expect("runtime"),
        ServerBootId::try_from(3).expect("server boot"),
    )
}

pub(super) fn health(dir: &StateDir) -> Health {
    let boot = ovis_terminal_protocol::schema::ids::ServerBootId::new(
        runtime().server_boot_id().to_string(),
    );
    let ledger = RecoveryLedger::open(Incidents::new(&dir.0), (boot, TimestampMs::new(1)), None)
        .expect("ledger");
    Health::new(runtime(), ledger)
}

pub(super) fn metrics(health: &Health) -> ServerMetricsResult {
    let observed = Observed::new(EventSeq::new(9), Vec::new());
    health.metrics(&observed).expect("metrics")
}

fn viewer(n: u8) -> ConsumerRef {
    ConsumerRef::Stream {
        stream_id: StreamId::new(format!("stream-{n}")),
        terminal_id: TerminalId::new("t1".to_owned()),
    }
}

fn client() -> ClientId {
    ClientId::new("ovis".to_owned())
}

fn bytes(first: u64, last: u64) -> LossRange {
    LossRange::Bytes {
        first_offset: ByteOffset::new(first),
        last_offset: ByteOffset::new(last),
    }
}

fn row(metrics: &ServerMetricsResult) -> &ConsumerQueue {
    metrics.consumers.first().expect("one consumer")
}

#[test]
fn a_stalled_viewer_shows_its_backlog_and_lag_until_written() {
    let dir = StateDir::new();
    let health = health(&dir);
    let gauge = health
        .consumers()
        .track(client(), viewer(1))
        .expect("tracked");
    gauge.queued(4096, TimestampMs::new(1));
    gauge.queued(100, TimestampMs::new(u64::MAX));
    let stalled = metrics(&health);
    assert_eq!((stalled.streams.get(), stalled.subscriptions.get()), (1, 0));
    assert_eq!(row(&stalled).queue_bytes.get(), 4196);
    assert!(row(&stalled).event_lag_ms.get() > 0);
    gauge.written(4096);
    let drained = metrics(&health);
    assert_eq!(row(&drained).queue_bytes.get(), 100);
    assert_eq!(
        row(&drained).event_lag_ms.get(),
        0,
        "the oldest unwritten item is the future one"
    );
    gauge.written(u64::MAX);
    assert_eq!(row(&metrics(&health)).queue_bytes.get(), 0);
}

#[test]
fn dropped_ranges_stay_until_resynced_and_overflow_is_counted() {
    let dir = StateDir::new();
    let health = health(&dir);
    let gauge = health
        .consumers()
        .track(client(), viewer(1))
        .expect("tracked");
    gauge.lost(bytes(0, 99));
    let lost = metrics(&health);
    assert_eq!(
        row(&lost).losses.first().map(|loss| loss.resynced),
        Some(false)
    );
    assert_eq!(
        row(&metrics(&health)).losses,
        row(&lost).losses,
        "a read changes nothing"
    );
    gauge.resynced(bytes(0, 99));
    let resynced = metrics(&health);
    assert_eq!(
        row(&resynced).losses.first().map(|loss| loss.resynced),
        Some(true)
    );
    for first in 1..=u64::try_from(MAX_LOSSES).expect("bound") {
        gauge.lost(bytes(first * 100, first * 100 + 99));
    }
    let full = metrics(&health);
    assert_eq!(row(&full).losses.len(), MAX_LOSSES);
    assert_eq!(row(&full).losses_omitted.get(), 1);
}

#[test]
fn rows_follow_gauge_lifetimes_and_the_table_is_bounded() {
    let dir = StateDir::new();
    let health = health(&dir);
    let subscription = ConsumerRef::Subscription {
        subscription_id: SubscriptionId::new("sub-1".to_owned()),
    };
    let first = health
        .consumers()
        .track(client(), subscription.clone())
        .expect("tracked");
    let again = health.consumers().track(client(), subscription);
    assert!(matches!(again, Err(HealthError::AlreadyTracked)));
    drop(first);
    assert_eq!(metrics(&health).consumers, Vec::new());
    let mut gauges = Vec::new();
    for n in 0..MAX_CONSUMERS {
        let consumer = ConsumerRef::Subscription {
            subscription_id: SubscriptionId::new(format!("sub-{n}")),
        };
        gauges.push(
            health
                .consumers()
                .track(client(), consumer)
                .expect("within the bound"),
        );
    }
    assert_eq!(gauges.len(), MAX_CONSUMERS);
    let refused = health
        .consumers()
        .track(client(), viewer(9))
        .expect_err("table is full");
    assert_eq!(refused.kind(), "consumer_capacity");
    let full = metrics(&health);
    assert_eq!(full.consumers.len(), MAX_CONSUMERS);
    assert_eq!(
        full.subscriptions.get(),
        u64::try_from(MAX_CONSUMERS).expect("bound")
    );
    assert_eq!(full.consumers_untracked.get(), 1);
}

#[test]
fn latency_is_kept_per_catalog_method_with_its_slowest_operation() {
    let latencies = Latencies::new();
    let settle = |method, millis, (failed, op): (bool, Option<&str>)| {
        let op_id = op.map(|op| OpId::new(op.to_owned()));
        Settled::new((method, Duration::from_millis(millis)), (failed, op_id))
    };
    latencies.record(settle("doc.set", 5, (false, Some("op-a"))));
    latencies.record(settle("doc.set", 40, (true, Some("op-b"))));
    latencies.record(settle("doc.set", 7, (false, None)));
    latencies.record(settle("no.such.method", 1, (false, None)));
    let rows = latencies.rows();
    assert_eq!(rows.len(), 1, "names outside the catalog are not recorded");
    let row = rows.first().expect("one row");
    assert_eq!(row.method.as_str(), "doc.set");
    assert_eq!((row.calls.get(), row.failures.get()), (3, 1));
    assert_eq!((row.last_ms.get(), row.max_ms.get()), (7, 40));
    assert_eq!(row.slowest_op, Some(OpId::new("op-b".to_owned())));
}

#[test]
fn metrics_name_the_current_lifetime_and_boot() {
    let dir = StateDir::new();
    let health = health(&dir);
    let read = metrics(&health);
    assert_eq!(
        read.runtime_lifetime.as_str(),
        runtime().runtime_id().to_string()
    );
    assert_eq!(
        read.server_boot_id.as_str(),
        runtime().server_boot_id().to_string()
    );
    assert_eq!(read.latest_seq, EventSeq::new(9));
    let status = health.status(&[]);
    assert_eq!(status.server_boot_id, Some(read.server_boot_id));
    assert_eq!(status.census, OpenEnum::Known(CensusState::NotReported));
    assert_eq!((status.holder_count, status.live_terminals), (None, None));
    health.publish_census(Census::new(2, 3));
    let census = health.status(&[]);
    assert_eq!(census.census, OpenEnum::Known(CensusState::Reported));
    assert_eq!(
        (census.holder_count, census.live_terminals),
        (Some(Count::new(2)), Some(Count::new(3)))
    );
}
