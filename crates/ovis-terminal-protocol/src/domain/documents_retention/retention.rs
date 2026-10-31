//! Retention classes, advertised limits and the floors eviction leaves behind.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::rpc::DecimalU64;
use crate::schema::limits::Limits;
use crate::schema::scalar::{DurationMs, Revision};

/// Data whose eviction is advertised and leaves a visible floor.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RetentionClass {
    /// Raw terminal output and its history lines.
    OutputHistory,
    /// Replayable events.
    Events,
    /// Operation receipts that deduplicate retried mutations.
    Receipts,
    /// Plugin run records.
    PluginRuns,
    /// Metadata of finished terminals.
    FinishedTerminals,
}

impl RetentionClass {
    /// Every class, in advertisement order.
    pub const ALL: [Self; 5] = [
        Self::OutputHistory,
        Self::Events,
        Self::Receipts,
        Self::PluginRuns,
        Self::FinishedTerminals,
    ];

    /// Stable token, equal to the wire form.
    pub const fn token(self) -> &'static str {
        match self {
            Self::OutputHistory => "output_history",
            Self::Events => "events",
            Self::Receipts => "receipts",
            Self::PluginRuns => "plugin_runs",
            Self::FinishedTerminals => "finished_terminals",
        }
    }
}

/// What a retention capacity counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RetentionUnit {
    /// Bytes of retained data.
    Bytes,
    /// Retained records.
    Records,
}

/// A position in one class's own monotonic coordinate: output byte offset, event
/// sequence, receipt revision, plugin run number or finished-terminal sequence.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct RetentionPosition(DecimalU64);

impl RetentionPosition {
    /// Wraps a native position.
    pub const fn new(value: u64) -> Self {
        Self(DecimalU64::new(value))
    }

    /// The native position.
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// A retained quantity in a class's [`RetentionUnit`]: bytes or records.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct RetentionQuantity(DecimalU64);

impl RetentionQuantity {
    /// Wraps a native quantity.
    pub const fn new(value: u64) -> Self {
        Self(DecimalU64::new(value))
    }

    /// The native quantity.
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// The advertised bound of one class; data past it is evicted behind the floor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct RetentionLimit {
    /// Class bounded by this limit.
    pub class: RetentionClass,
    /// What `capacity` counts.
    pub unit: RetentionUnit,
    /// Retained quantity before the oldest data is evicted.
    pub capacity: RetentionQuantity,
    /// Age after which data is evicted; absent when only capacity bounds the class.
    pub max_age_ms: Option<DurationMs>,
}

impl RetentionLimit {
    /// A capacity-only bound.
    pub const fn new(class: RetentionClass, unit: RetentionUnit, capacity: u64) -> Self {
        Self {
            class,
            unit,
            capacity: RetentionQuantity::new(capacity),
            max_age_ms: None,
        }
    }

    /// Adds an age bound.
    #[must_use]
    pub const fn max_age(mut self, age: DurationMs) -> Self {
        self.max_age_ms = Some(age);
        self
    }
}

/// Retention bounds of every class, advertised so clients never hard-code them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct RetentionPolicy {
    /// One limit per class, in [`RetentionClass::ALL`] order.
    pub limits: Vec<RetentionLimit>,
}

const WEEK_MS: u64 = 7 * 24 * 60 * 60 * 1000;

impl RetentionPolicy {
    /// The v1 defaults, consistent with [`Limits::v1`].
    pub fn v1() -> Self {
        let limits = Limits::v1();
        let week = DurationMs::new(WEEK_MS);
        Self {
            limits: vec![
                RetentionLimit::new(
                    RetentionClass::OutputHistory,
                    RetentionUnit::Bytes,
                    limits.output_ring_bytes.get(),
                ),
                RetentionLimit::new(
                    RetentionClass::Events,
                    RetentionUnit::Records,
                    u64::from(limits.event_replay_events),
                ),
                RetentionLimit::new(RetentionClass::Receipts, RetentionUnit::Records, 100_000)
                    .max_age(limits.op_retention_ms),
                RetentionLimit::new(RetentionClass::PluginRuns, RetentionUnit::Records, 1000)
                    .max_age(week),
                RetentionLimit::new(
                    RetentionClass::FinishedTerminals,
                    RetentionUnit::Records,
                    1000,
                )
                .max_age(week),
            ],
        }
    }

    /// Replaces the limit of `limit.class`.
    #[must_use]
    pub fn with_limit(mut self, limit: RetentionLimit) -> Self {
        self.limits.retain(|existing| existing.class != limit.class);
        self.limits.push(limit);
        self
    }

    /// The limit of `class`.
    pub fn limit(&self, class: RetentionClass) -> Option<&RetentionLimit> {
        self.limits.iter().find(|limit| limit.class == class)
    }
}

/// Whether retained data still covers a position or a cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Retained {
    /// The position is retained.
    Available,
    /// The position was evicted; it existed and is not merely absent.
    Expired {
        /// Oldest retained position.
        floor: RetentionPosition,
    },
    /// Positions after the cursor were evicted; resume at the floor.
    Gap {
        /// Oldest retained position.
        floor: RetentionPosition,
    },
}

/// The oldest retained position of one class; everything below it expired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct RetentionFloor {
    /// Class this floor belongs to.
    pub class: RetentionClass,
    /// Oldest retained position; zero when nothing was evicted.
    pub floor: RetentionPosition,
}

impl RetentionFloor {
    /// The floor of `class`.
    pub const fn new(class: RetentionClass, floor: RetentionPosition) -> Self {
        Self { class, floor }
    }

    /// A point read at `position`: below the floor it is expired.
    pub const fn lookup(self, position: RetentionPosition) -> Retained {
        if position.get() < self.floor.get() {
            Retained::Expired { floor: self.floor }
        } else {
            Retained::Available
        }
    }

    /// Resuming after `cursor`: a gap when the next position was evicted.
    pub const fn resume_after(self, cursor: RetentionPosition) -> Retained {
        match cursor.get().checked_add(1) {
            Some(next) if next < self.floor.get() => Retained::Gap { floor: self.floor },
            Some(_) | None => Retained::Available,
        }
    }
}

/// Floors of every class read at one store revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct RetentionFloors {
    /// Store revision the floors were read at.
    pub revision: Revision,
    /// One floor per class, in [`RetentionClass::ALL`] order.
    pub floors: Vec<RetentionFloor>,
}

impl RetentionFloors {
    /// Floors read at `revision`.
    pub const fn new(revision: Revision, floors: Vec<RetentionFloor>) -> Self {
        Self { revision, floors }
    }

    /// The floor of `class`.
    pub fn floor(&self, class: RetentionClass) -> Option<RetentionFloor> {
        self.floors
            .iter()
            .copied()
            .find(|floor| floor.class == class)
    }
}
