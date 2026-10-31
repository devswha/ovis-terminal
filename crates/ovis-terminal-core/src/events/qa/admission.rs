//! FIFO admission to the pinned-snapshot budget.

use std::sync::mpsc;
use std::time::Duration;

use ovis_terminal_core::events::{Coordinator, EventError, SnapshotLease};
use ovis_terminal_protocol::schema::scalar::QueuePosition;

use super::{Changed, Fixture, Producer, StoreOptions, TestResult, identity};

type Core = Coordinator<Changed, Producer>;

fn open(fixture: &Fixture) -> Result<Core, Box<dyn std::error::Error>> {
    Ok(Coordinator::new(
        fixture.open(StoreOptions::default())?,
        identity(1)?,
        Producer,
    ))
}

fn pin_all(core: &Core) -> Result<Vec<SnapshotLease>, EventError> {
    let mut leases = Vec::new();
    loop {
        match core.pin_snapshot() {
            Ok(lease) => leases.push(lease),
            Err(EventError::LimitExceeded) => return Ok(leases),
            Err(error) => return Err(error),
        }
    }
}

#[test]
fn released_slots_reach_waiters_in_arrival_order() -> TestResult {
    // Given every slot held and five readers queued in a known order.
    let fixture = Fixture::new();
    let core = open(&fixture)?;
    let mut leases = pin_all(&core)?;
    assert_eq!(leases.len(), 8);
    let queue = core.snapshot_queue();
    let tickets: Vec<_> = (0..5).map(|_| queue.enqueue()).collect();
    let positions: Vec<_> = tickets
        .iter()
        .map(|ticket| queue.position(ticket.id()).map(QueuePosition::get))
        .collect();
    assert_eq!(positions, vec![Some(1), Some(2), Some(3), Some(4), Some(5)]);

    // When waiters start in reverse order and a single slot is released.
    let (granted, order) = mpsc::channel();
    std::thread::scope(|scope| -> TestResult {
        for (index, ticket) in tickets.into_iter().enumerate().rev() {
            let granted = granted.clone();
            scope.spawn(move || {
                let slot = ticket.wait();
                granted.send((index, slot.is_ok())).expect("observer");
            });
        }
        let lease = leases.pop().ok_or("lease")?;
        core.release_snapshot(&lease.token)?;
        let mut observed = Vec::new();
        for _ in 0..5 {
            observed.push(order.recv_timeout(Duration::from_secs(10))?);
        }

        // Then the one slot passes from waiter to waiter in arrival order.
        assert_eq!(
            observed,
            vec![(0, true), (1, true), (2, true), (3, true), (4, true)]
        );
        Ok(())
    })?;
    for lease in leases {
        core.release_snapshot(&lease.token)?;
    }
    assert_eq!(pin_all(&core)?.len(), 8);
    core.close()?;
    Ok(())
}

#[test]
fn a_fail_fast_pin_cannot_take_the_slot_a_waiter_is_owed() -> TestResult {
    // Given every slot held and one reader queued.
    let fixture = Fixture::new();
    let core = open(&fixture)?;
    let mut leases = pin_all(&core)?;
    let ticket = core.snapshot_queue().enqueue();

    // When a slot is released and a fail-fast pin arrives before the reader takes it.
    let lease = leases.pop().ok_or("lease")?;
    core.release_snapshot(&lease.token)?;
    let jumped = core.pin_snapshot();

    // Then the pin is refused and the queued reader still gets the slot.
    assert_eq!(jumped.expect_err("queue jump").kind(), "limit_exceeded");
    let slot = ticket.wait()?;
    assert_eq!(
        core.pin_snapshot().expect_err("slot is held").kind(),
        "limit_exceeded"
    );
    drop(slot);
    leases.push(core.pin_snapshot()?);
    assert_eq!(leases.len(), 8);
    for lease in leases {
        core.release_snapshot(&lease.token)?;
    }
    core.close()?;
    Ok(())
}

#[test]
fn withdrawn_and_dropped_tickets_leave_the_line() -> TestResult {
    // Given every slot held and three readers queued.
    let fixture = Fixture::new();
    let core = open(&fixture)?;
    let mut leases = pin_all(&core)?;
    let queue = core.snapshot_queue();
    let (withdrawn, dropped, last) = (queue.enqueue(), queue.enqueue(), queue.enqueue());

    // When the first is withdrawn and the second abandons its place.
    queue.withdraw(withdrawn.id());
    drop(dropped);

    // Then the last moves to the front and the withdrawn waiter learns why it left.
    assert_eq!(queue.position(last.id()).map(QueuePosition::get), Some(1));
    assert_eq!(queue.position(withdrawn.id()), None);
    assert_eq!(
        withdrawn.wait().expect_err("withdrawn").kind(),
        "unavailable"
    );
    let lease = leases.pop().ok_or("lease")?;
    core.release_snapshot(&lease.token)?;
    drop(last.wait()?);
    for lease in leases {
        core.release_snapshot(&lease.token)?;
    }
    assert_eq!(pin_all(&core)?.len(), 8);
    core.close()?;
    Ok(())
}
