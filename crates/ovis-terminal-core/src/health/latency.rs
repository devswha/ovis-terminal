use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use ovis_terminal_protocol::domain::health::MethodLatency;
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::MethodName;
use ovis_terminal_protocol::schema::methods::all_methods;
use ovis_terminal_protocol::schema::scalar::{Count, DurationMs};

/// One settled call as the dispatcher saw it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Settled<'a> {
    /// Requested method name, as received.
    pub method: &'a str,
    /// Time from receipt to the settled reply.
    pub elapsed: Duration,
    /// The reply was an error.
    pub failed: bool,
    /// Operation the request named, if any.
    pub op_id: Option<OpId>,
}

impl<'a> Settled<'a> {
    /// `method` settled after `elapsed`, `failed` or not, naming `op_id`.
    #[must_use]
    pub fn new(
        (method, elapsed): (&'a str, Duration),
        (failed, op_id): (bool, Option<OpId>),
    ) -> Self {
        Self {
            method,
            elapsed,
            failed,
            op_id,
        }
    }
}

#[derive(Debug, Default)]
struct Row {
    calls: u64,
    failures: u64,
    last_ms: u64,
    max_ms: u64,
    slowest_op: Option<OpId>,
}

/// Per-method latency since this boot, keyed only by catalog methods so the table is
/// bounded by the published catalog whatever names clients send.
#[derive(Debug, Default)]
pub struct Latencies {
    rows: Mutex<BTreeMap<&'static str, Row>>,
}

impl Latencies {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records `settled`; a name outside the catalog is not a method and is not recorded.
    pub fn record(&self, settled: Settled<'_>) {
        let Some(name) = all_methods()
            .map(|spec| spec.name)
            .find(|name| *name == settled.method)
        else {
            return;
        };
        let millis = u64::try_from(settled.elapsed.as_millis()).unwrap_or(u64::MAX);
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        let row = rows.entry(name).or_default();
        row.calls = row.calls.saturating_add(1);
        row.failures = row.failures.saturating_add(u64::from(settled.failed));
        row.last_ms = millis;
        if row.calls == 1 || millis > row.max_ms {
            row.max_ms = millis;
            row.slowest_op = settled.op_id;
        }
        drop(rows);
    }

    /// Copies every row, ordered by method name.
    #[must_use]
    pub fn rows(&self) -> Vec<MethodLatency> {
        let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        rows.iter()
            .map(|(name, row)| {
                MethodLatency::new(
                    MethodName::new((*name).to_owned()),
                    (Count::new(row.calls), Count::new(row.failures)),
                    (
                        DurationMs::new(row.last_ms),
                        DurationMs::new(row.max_ms),
                        row.slowest_op.clone(),
                    ),
                )
            })
            .collect()
    }
}
