//! Read-only health and per-consumer metrics (plan todo 59).
//!
//! Producers publish into bounded tables: a stream viewer or subscription pump owns a
//! [`ConsumerGauge`] and reports queued, written, dropped and resynchronized ranges; the
//! dispatcher wrapper reports each settled call to [`Latencies`]; recovery publishes its
//! facts once at boot and on every recorded incident through [`RecoveryLedger`]. A read
//! copies those published values and performs no I/O of its own, so collecting metrics
//! never waits on a producer's work and never changes what it reads: no counter resets,
//! no incident is acknowledged, no server is started.

mod consumers;
mod error;
mod latency;
mod monitor;
#[cfg(test)]
mod qa;
mod recovery;

pub use consumers::{ConsumerGauge, Consumers, MAX_CONSUMERS, MAX_LOSSES};
pub use error::HealthError;
pub use latency::{Latencies, Settled};
pub use monitor::{Census, Health, Observed};
pub use recovery::RecoveryLedger;
