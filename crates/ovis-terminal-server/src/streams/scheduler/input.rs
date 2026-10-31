//! One terminal's accepted input: reserved entries in admission order, one being written.

use std::collections::VecDeque;
use std::fmt;

use super::budget::Reservation;

/// Admission order of one input entry on its terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ticket(u64);

impl Ticket {
    /// The admission sequence number.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Bytes reserved against the terminal and session input budgets, not yet admitted.
#[derive(Debug)]
pub struct Reserved {
    /// The terminal reservation, then the session reservation of the same bytes.
    pub(super) holds: [Reservation; 2],
}

impl Reserved {
    /// Bytes reserved.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        let [terminal, _] = &self.holds;
        terminal.bytes()
    }
}

/// What a cancellation found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CancelOutcome {
    /// The entry was withdrawn before any byte of it was written.
    Withdrawn,
    /// The entry is being written; bytes already written cannot be retracted.
    InFlight,
    /// No queued or in-flight entry has this ticket.
    NotQueued,
}

#[derive(Debug)]
pub(super) struct Entry {
    pub ticket: Ticket,
    pub bytes: Vec<u8>,
    pub held: Reserved,
}

#[derive(Debug, Default)]
pub(super) struct InputQueue {
    entries: VecDeque<Entry>,
    writing: Option<Ticket>,
    next: u64,
    closed: bool,
}

pub(super) enum Next {
    Write(Entry),
    Idle,
    Closed,
}

impl InputQueue {
    pub(super) fn admit(&mut self, bytes: Vec<u8>, held: Reserved) -> Ticket {
        self.next = self.next.saturating_add(1);
        let ticket = Ticket(self.next);
        self.entries.push_back(Entry {
            ticket,
            bytes,
            held,
        });
        ticket
    }

    pub(super) fn cancel(&mut self, ticket: Ticket) -> CancelOutcome {
        if self.writing == Some(ticket) {
            return CancelOutcome::InFlight;
        }
        match self.entries.iter().position(|entry| entry.ticket == ticket) {
            Some(index) => {
                self.entries.remove(index);
                CancelOutcome::Withdrawn
            }
            None => CancelOutcome::NotQueued,
        }
    }

    pub(super) fn take(&mut self) -> Next {
        if self.closed {
            return Next::Closed;
        }
        match self.entries.pop_front() {
            Some(entry) => {
                self.writing = Some(entry.ticket);
                Next::Write(entry)
            }
            None => Next::Idle,
        }
    }

    pub(super) const fn written(&mut self) {
        self.writing = None;
    }

    /// Stops writing; returns the entries never started and the one cut off mid-write.
    pub(super) fn close(&mut self) -> (usize, Option<Ticket>) {
        self.closed = true;
        let unsent = self.entries.len();
        self.entries.clear();
        (unsent, self.writing.take())
    }

    pub(super) fn queued(&self) -> usize {
        self.entries.len()
    }
}
