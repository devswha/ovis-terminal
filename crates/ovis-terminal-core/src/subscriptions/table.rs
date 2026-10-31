//! In-memory subscription entries guarded by the hub lock; no I/O happens here.

use std::collections::VecDeque;

use ovis_terminal_protocol::schema::ids::SubscriptionId;
use ovis_terminal_protocol::schema::scalar::EventSeq;

use super::{LossCause, LossRecord, QueueLimits, Selector, SubscriptionError};
use crate::events::Event;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Lost {
    pub from: EventSeq,
    pub cause: LossCause,
    pub persisted: bool,
}

impl Lost {
    pub(super) fn record(self, id: &SubscriptionId) -> LossRecord {
        LossRecord::new(id.clone(), self.from, self.cause)
    }
}

/// Why a queue is replaced by a loss marker, and the sequence that was being offered.
#[derive(Debug, Clone, Copy)]
pub(super) struct Breach {
    pub cause: LossCause,
    pub at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    Active,
    Lost(Lost),
    Closed,
}

#[derive(Debug)]
pub(super) struct Entry<D> {
    pub id: SubscriptionId,
    pub selector: Selector,
    pub scan: u64,
    pub delivered: u64,
    pub queue: VecDeque<(Event<D>, usize)>,
    pub bytes: usize,
    pub phase: Phase,
    pub orphaned: bool,
}

impl<D> Entry<D> {
    pub(super) const fn new(id: SubscriptionId, selector: Selector, after: u64) -> Self {
        Self {
            id,
            selector,
            scan: after,
            delivered: after,
            queue: VecDeque::new(),
            bytes: 0,
            phase: Phase::Active,
            orphaned: false,
        }
    }

    /// The loss this entry still owes or already committed, counting undelivered events.
    pub(super) fn owed(&self) -> Option<LossRecord> {
        match self.phase {
            Phase::Lost(lost) => Some(lost.record(&self.id)),
            Phase::Active => {
                let (front, _) = self.queue.front()?;
                Some(LossRecord::new(
                    self.id.clone(),
                    front.cursor.seq,
                    LossCause::ForcedClose,
                ))
            }
            Phase::Closed => None,
        }
    }
}

/// Every open subscription plus the highest sequence already fanned out.
#[derive(Debug)]
pub(super) struct Table<D> {
    pub entries: Vec<Entry<D>>,
    pub head: u64,
    pub session_bytes: usize,
    pub limits: QueueLimits,
}

impl<D> Table<D> {
    pub(super) const fn new(head: u64, limits: QueueLimits) -> Self {
        Self {
            entries: Vec::new(),
            head,
            session_bytes: 0,
            limits,
        }
    }

    pub(super) fn index(&self, id: &SubscriptionId) -> Result<usize, SubscriptionError> {
        self.entries
            .iter()
            .position(|entry| entry.id == *id)
            .ok_or(SubscriptionError::NotFound)
    }

    pub(super) fn find(&mut self, id: &SubscriptionId) -> Result<&mut Entry<D>, SubscriptionError> {
        self.entries
            .iter_mut()
            .find(|entry| entry.id == *id && !entry.orphaned && entry.phase != Phase::Closed)
            .ok_or(SubscriptionError::NotFound)
    }

    pub(super) fn reserve(&self) -> Result<(), SubscriptionError> {
        let open = self
            .entries
            .iter()
            .filter(|entry| entry.phase != Phase::Closed)
            .count();
        if open >= self.limits.subscriptions {
            return Err(SubscriptionError::Limit);
        }
        Ok(())
    }

    pub(super) fn insert(&mut self, entry: Entry<D>) -> Result<usize, SubscriptionError> {
        self.reserve()?;
        self.entries.push(entry);
        Ok(self.entries.len().saturating_sub(1))
    }

    /// Replaces the queue with a loss marker; the first undelivered matching sequence is lost.
    pub(super) fn lose(&mut self, index: usize, breach: Breach) {
        if let Some(entry) = self.entries.get_mut(index) {
            let from = entry
                .queue
                .front()
                .map_or(breach.at, |(event, _)| event.cursor.seq.get());
            self.session_bytes = self.session_bytes.saturating_sub(entry.bytes);
            entry.queue.clear();
            entry.bytes = 0;
            entry.phase = Phase::Lost(Lost {
                from: EventSeq::new(from),
                cause: breach.cause,
                persisted: false,
            });
        }
    }

    pub(super) fn lose_all(&mut self, cause: LossCause) {
        let breach = Breach {
            cause,
            at: self.head.saturating_add(1),
        };
        for index in 0..self.entries.len() {
            if self
                .entries
                .get(index)
                .is_some_and(|entry| entry.phase == Phase::Active)
            {
                self.lose(index, breach);
            }
        }
    }

    pub(super) fn owes(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| matches!(entry.phase, Phase::Lost(lost) if !lost.persisted))
    }

    pub(super) fn unpersisted(&self) -> Vec<LossRecord> {
        self.entries
            .iter()
            .filter_map(|entry| match entry.phase {
                Phase::Lost(lost) if !lost.persisted => Some(lost.record(&entry.id)),
                Phase::Active | Phase::Lost(_) | Phase::Closed => None,
            })
            .collect()
    }

    /// Marks committed records and removes orphans that no longer owe a marker.
    pub(super) fn persisted(&mut self, records: &[LossRecord]) {
        for entry in &mut self.entries {
            if let Phase::Lost(lost) = &mut entry.phase {
                lost.persisted |= records.iter().any(|record| {
                    record.subscription_id == entry.id && record.from_seq == lost.from
                });
            }
        }
        self.retain(|entry| {
            !entry.orphaned || matches!(entry.phase, Phase::Lost(lost) if !lost.persisted)
        });
    }

    /// Detaches an entry whose consumer is gone; undelivered events become a forced-close loss.
    pub(super) fn orphan(&mut self, id: &SubscriptionId) {
        let Ok(index) = self.index(id) else {
            return;
        };
        let pending = self
            .entries
            .get(index)
            .is_some_and(|entry| entry.phase == Phase::Active && !entry.queue.is_empty());
        if pending {
            self.lose(
                index,
                Breach {
                    cause: LossCause::ForcedClose,
                    at: 0,
                },
            );
        }
        if let Some(entry) = self.entries.get_mut(index) {
            entry.orphaned = true;
        }
        self.persisted(&[]);
    }

    /// Drops an entry that never reached its consumer.
    pub(super) fn remove(&mut self, id: &SubscriptionId) {
        self.retain(|entry| entry.id != *id);
    }

    fn retain(&mut self, keep: impl Fn(&Entry<D>) -> bool) {
        let mut released = 0_usize;
        self.entries.retain(|entry| {
            let kept = keep(entry);
            if !kept {
                released = released.saturating_add(entry.bytes);
            }
            kept
        });
        self.session_bytes = self.session_bytes.saturating_sub(released);
    }
}
