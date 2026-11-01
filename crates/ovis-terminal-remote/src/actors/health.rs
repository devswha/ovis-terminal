//! Publishing an actor's failure state; every change advances its revision.

use ovis_terminal_protocol::domain::machine_actor::{LineFailure, MachineHealth};
use ovis_terminal_protocol::errors::ErrorKind;
use ovis_terminal_protocol::schema::methods::machine::MachineState;
use ovis_terminal_protocol::schema::scalar::{Count, EventSeq, OpenEnum, Revision, TimestampMs};
use tokio::sync::watch;

/// The local wall clock in milliseconds; a clock before 1970 reads as zero.
pub(super) fn now_ms() -> TimestampMs {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    let millis = since.map_or(0, |since| since.as_millis());
    TimestampMs::new(u64::try_from(millis).unwrap_or(u64::MAX))
}

const fn bump(count: Count, by: u64) -> Count {
    Count::new(count.get().saturating_add(by))
}

/// One actor's published health.
#[derive(Debug)]
pub(super) struct Health(watch::Sender<MachineHealth>);

impl Health {
    pub(super) fn new(health: MachineHealth) -> (Self, watch::Receiver<MachineHealth>) {
        let (sender, receiver) = watch::channel(health);
        (Self(sender), receiver)
    }

    pub(super) fn get(&self) -> MachineHealth {
        self.0.borrow().clone()
    }

    fn change(&self, edit: impl FnOnce(&mut MachineHealth)) {
        self.0.send_modify(|health| {
            edit(health);
            health.revision = Revision::new(health.revision.get().saturating_add(1));
        });
    }

    pub(super) fn attached(&self) {
        self.change(|health| {
            health.state = OpenEnum::Known(MachineState::Connected);
            health.line_epoch = bump(health.line_epoch, 1);
            health.in_flight = Count::new(0);
        });
    }

    pub(super) fn lost(&self, kind: &'static str) {
        self.change(|health| {
            health.state = OpenEnum::Known(MachineState::Unreachable);
            health.last_failure = Some(failure(kind));
            health.in_flight = Count::new(0);
        });
    }

    pub(super) fn expired(&self, (expired, in_flight): (u64, usize)) {
        self.change(|health| {
            health.deadline_exceeded = bump(health.deadline_exceeded, expired);
            health.last_failure = Some(failure("deadline_exceeded"));
            health.in_flight = count(in_flight);
        });
    }

    /// Records the number of calls in flight without advancing the revision when unchanged.
    pub(super) fn in_flight(&self, in_flight: usize) {
        if self.0.borrow().in_flight != count(in_flight) {
            self.change(|health| health.in_flight = count(in_flight));
        }
    }

    pub(super) fn seen(&self, seq: EventSeq) {
        self.change(|health| health.last_seen_seq = Some(seq));
    }

    pub(super) fn dropped(&self) {
        self.change(|health| health.inbound_dropped = bump(health.inbound_dropped, 1));
    }
}

fn count(value: usize) -> Count {
    Count::new(u64::try_from(value).unwrap_or(u64::MAX))
}

fn failure(kind: &'static str) -> LineFailure {
    LineFailure::new(ErrorKind::from(kind.to_owned()), now_ms())
}
