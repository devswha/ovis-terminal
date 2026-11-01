//! A bounded, redacted audit trail with an explicit retention policy.

use std::collections::VecDeque;
use std::num::NonZeroUsize;

use ovis_terminal_protocol::schema::scalar::TimestampMs;
use serde_json::Value;

use super::rules::redact;

/// How long and how much of the audit trail is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    max_records: NonZeroUsize,
    max_age_ms: u64,
}

impl RetentionPolicy {
    /// Keeps at most `max_records`, none older than `max_age_ms`.
    #[must_use]
    pub const fn new(max_records: NonZeroUsize, max_age_ms: u64) -> Self {
        Self {
            max_records,
            max_age_ms,
        }
    }

    /// The record bound.
    #[must_use]
    pub const fn max_records(&self) -> NonZeroUsize {
        self.max_records
    }

    /// The age bound in milliseconds.
    #[must_use]
    pub const fn max_age_ms(&self) -> u64 {
        self.max_age_ms
    }
}

/// Audit records retained after redaction, oldest first.
///
/// Records enter only through [`AuditLog::append`], which redacts them first, so the trail
/// never holds a second copy of a credential or secret input. Each object record is numbered
/// with `seq`, a decimal string counting every append, so a reader sees which records the
/// policy evicted. Who may read the trail is decided by the caller's authorization; the log
/// only keeps what the policy allows.
#[derive(Debug)]
pub struct AuditLog {
    policy: RetentionPolicy,
    records: VecDeque<(TimestampMs, Value)>,
    appended: u64,
    evicted: u64,
}

impl AuditLog {
    /// An empty trail under `policy`.
    #[must_use]
    pub const fn new(policy: RetentionPolicy) -> Self {
        Self {
            policy,
            records: VecDeque::new(),
            appended: 0,
            evicted: 0,
        }
    }

    /// Redacts `record`, retains it as of `at` and evicts what the policy no longer allows.
    pub fn append(&mut self, (at, mut record): (TimestampMs, Value)) {
        redact(&mut record);
        self.appended = self.appended.saturating_add(1);
        if let Value::Object(fields) = &mut record {
            fields.insert("seq".to_owned(), Value::String(self.appended.to_string()));
        }
        self.records.push_back((at, record));
        self.prune(at);
    }

    /// Evicts records older than the age bound at `now`, then any beyond the record bound.
    pub fn prune(&mut self, now: TimestampMs) {
        let horizon = now.get().saturating_sub(self.policy.max_age_ms);
        while self
            .records
            .front()
            .is_some_and(|(at, _)| at.get() < horizon)
        {
            self.records.pop_front();
            self.evicted = self.evicted.saturating_add(1);
        }
        while self.records.len() > self.policy.max_records.get() {
            self.records.pop_front();
            self.evicted = self.evicted.saturating_add(1);
        }
    }

    /// The retained records at `now`, oldest first.
    pub fn records(&mut self, now: TimestampMs) -> Vec<Value> {
        self.prune(now);
        self.records
            .iter()
            .map(|(_, record)| record.clone())
            .collect()
    }

    /// How many records the policy has evicted, so a reader knows the trail is partial.
    #[must_use]
    pub const fn evicted(&self) -> u64 {
        self.evicted
    }
}
