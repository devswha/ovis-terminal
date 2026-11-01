//! Scanning one committed event into every independent subscription queue.

use super::LossCause;
use super::table::{Breach, Phase, Table};
use crate::events::Event;

/// One committed event with its serialized size, charged once per queue copy.
#[derive(Debug)]
pub(super) struct Offered<'a, D> {
    pub event: &'a Event<D>,
    pub bytes: usize,
}

impl<D> Clone for Offered<'_, D> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<D> Copy for Offered<'_, D> {}

impl<D> Offered<'_, D> {
    const fn seq(&self) -> u64 {
        self.event.cursor.seq.get()
    }

    const fn breach(&self, cause: LossCause) -> Breach {
        Breach {
            cause,
            at: self.seq(),
        }
    }
}

impl<D: Clone> Table<D> {
    /// Examines one event for every active subscription in commit order.
    ///
    /// Each subscription advances its own scan cursor, so a nonmatching event is visible
    /// as scanned rather than missing. A subscription whose own bound overflows loses
    /// alone; when the session budget runs out, the largest queue loses first.
    pub(super) fn fan_out(&mut self, offered: Offered<'_, D>) {
        for index in 0..self.entries.len() {
            self.offer(index, offered);
        }
        self.head = self.head.max(offered.seq());
    }

    /// Examines one event for one subscription; events at or below its scan cursor were
    /// already examined by its snapshot, replay or an earlier fan-out.
    pub(super) fn offer(&mut self, index: usize, offered: Offered<'_, D>) {
        let Some(entry) = self.entries.get_mut(index) else {
            return;
        };
        if entry.phase != Phase::Active || offered.seq() <= entry.scan {
            return;
        }
        entry.scan = offered.seq();
        if !entry.selector.matches(offered.event) {
            return;
        }
        let over = entry.queue.len() >= self.limits.queue_events
            || entry.bytes.saturating_add(offered.bytes) > self.limits.queue_bytes;
        if over {
            self.lose(index, offered.breach(LossCause::QueueOverflow));
        } else if self.make_room(index, offered) {
            self.push(index, offered);
        }
    }

    /// Evicts the largest queues until the event fits the session budget; false when the
    /// receiving subscription itself was the largest.
    fn make_room(&mut self, receiver: usize, offered: Offered<'_, D>) -> bool {
        while self.session_bytes.saturating_add(offered.bytes) > self.limits.session_bytes {
            let largest = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.phase == Phase::Active)
                .max_by_key(|(index, entry)| (entry.bytes, *index == receiver))
                .map(|(index, _)| index);
            match largest {
                Some(index) if index != receiver => {
                    self.lose(index, offered.breach(LossCause::SessionBudget));
                }
                Some(_) | None => {
                    self.lose(receiver, offered.breach(LossCause::SessionBudget));
                    return false;
                }
            }
        }
        true
    }

    fn push(&mut self, index: usize, offered: Offered<'_, D>) {
        if let Some(entry) = self.entries.get_mut(index) {
            entry
                .queue
                .push_back((offered.event.clone(), offered.bytes));
            entry.bytes = entry.bytes.saturating_add(offered.bytes);
            entry.report_queued(offered.bytes);
            self.session_bytes = self.session_bytes.saturating_add(offered.bytes);
        }
    }
}
