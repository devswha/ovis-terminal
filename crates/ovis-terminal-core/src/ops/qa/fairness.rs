//! Operations beyond the pinned-snapshot budget wait their turn instead of failing.

use std::sync::Barrier;

use ovis_terminal_protocol::errors::OpId;
use serde_json::json;

use super::{Fixture, OpState, OpenEnum, Operations, Request, TestResult, context};

fn writer(sequence: u128) -> Result<Request, Box<dyn std::error::Error>> {
    let timestamp = 100_000_000_u128 << 80;
    let version = 7_u128 << 76;
    let variant = 2_u128 << 62;
    let op_id = uuid::Uuid::from_u128(timestamp | version | variant | sequence).to_string();
    Ok(Request::new(
        OpId::new(op_id),
        "terminal.run".into(),
        json!({"argv":[format!("writer-{sequence}")]}),
    )?)
}

#[test]
fn thirty_two_concurrent_writers_all_commit_under_eight_pinned_snapshots() -> TestResult {
    // Given one runtime with the default budget of eight pinned snapshots.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    assert_eq!(
        ovis_terminal_core::events::Limits::default().pinned_snapshots,
        8
    );
    let barrier = Barrier::new(32);

    // When 32 writers accept, start and commit at the same instant.
    let outcomes = std::thread::scope(|scope| {
        let workers: Vec<_> = (1..=32_u128)
            .map(|sequence| {
                let (core, barrier) = (&core, &barrier);
                scope.spawn(
                    move || -> Result<_, Box<dyn std::error::Error + Send + Sync>> {
                        let request = writer(sequence).map_err(|error| error.to_string())?;
                        let operations =
                            Operations::new(core, context(1).map_err(|e| e.to_string())?);
                        barrier.wait();
                        operations.accept(&request)?;
                        let permit = operations.start(request.op_id())?;
                        Ok(operations.commit(permit, json!({"writer": sequence.to_string()}))?)
                    },
                )
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().map_err(|_| "writer panicked"))
            .collect::<Result<Vec<_>, _>>()
    })?;

    // Then every writer committed; none was refused at the budget.
    let mut committed = 0;
    for outcome in outcomes {
        let record = outcome.map_err(|error| format!("writer refused: {error}"))?;
        assert_eq!(record.state, OpenEnum::Known(OpState::Committed));
        committed += 1;
    }
    assert_eq!(committed, 32);
    core.close()?;
    Ok(())
}

#[test]
fn a_granted_slot_carries_the_accept_while_every_other_slot_is_held() -> TestResult {
    // Given every slot held and one request queued for the next.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let mut leases = Vec::new();
    while let Ok(lease) = core.pin_snapshot() {
        leases.push(lease);
    }
    assert_eq!(leases.len(), 8);
    let ticket = core.snapshot_queue().enqueue();
    let lease = leases.pop().ok_or("lease")?;
    core.release_snapshot(&lease.token)?;
    let slot = ticket.wait()?;

    // When the accept runs under that granted slot while seven others stay pinned.
    let operations = Operations::new(&core, context(1)?);
    let request = writer(1)?;
    let accepted = operations.accept_granted(&request, slot)?;

    // Then it needed no second slot, and its slot returned for the next read.
    assert_eq!(accepted.state, OpenEnum::Known(OpState::Accepted));
    assert_eq!(operations.get(request.op_id())?, accepted);
    for lease in leases {
        core.release_snapshot(&lease.token)?;
    }
    core.close()?;
    Ok(())
}
