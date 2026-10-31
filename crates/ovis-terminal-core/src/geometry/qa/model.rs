//! One transition's ordering, storm coalescing and unchanged requests.

use std::time::Duration;

use ovis_terminal_core::geometry::{GeometryEpoch, GeometryEvent, Outcome, Resolved};
use ovis_terminal_vt::Geometry;
use proptest::prelude::*;

use super::TestResult;
use super::rig::{Rig, dispatched, holder_target, op, screen, store_op};

const BOUND: Duration = Duration::from_secs(5);

#[test]
fn the_change_commits_before_the_grid_exists_and_ready_follows_the_acknowledgement() -> TestResult {
    // Given the operator owns geometry of an 80x24 screen and a subscriber is listening.
    let mut rig = Rig::new(screen(holder_target(1), 80, 24)?)?;
    let subscription = rig.core.subscribe(None)?;

    // When the operator asks for 100x30 and the transition runs step by step.
    let dispatch = dispatched(rig.resize(&op(), (100, 30))??)?;
    let epoch = dispatch.epoch();
    let (unit, committing) = dispatch.into_commit(store_op());
    let receipt = rig.core.commit(unit)?;
    let grid_at_commit = rig.screen.first_paint()?.geometry;
    let gate_at_commit = rig.geometry.input_ready().err().map(|error| error.kind());
    let resize = rig.geometry.apply(&committing, receipt)?;
    let gate_at_apply = rig.geometry.input_ready().err().map(|error| error.kind());
    let settled = rig.acknowledged(epoch)?;
    rig.publish(&settled)?;

    // Then the grid changes only after the change event committed, and the holder message
    // carries the new grid at the new epoch.
    assert_eq!(grid_at_commit, Geometry::new(80, 24)?);
    assert_eq!(resize.geometry, (100, 30));
    assert_eq!(resize.geometry_epoch, epoch.get());
    // And input waits for the runtime boundary, then the runtime itself declares it ready.
    assert_eq!(gate_at_commit, Some("not_ready"));
    assert_eq!(gate_at_apply, Some("not_ready"));
    assert_eq!(rig.geometry.input_ready()?, epoch);
    // And subscribers see changed, checkpoint, input_ready in consecutive commit order.
    let events = [(); 3].map(|()| subscription.delivery.recv_timeout(BOUND));
    let [changed, checkpoint, ready] = events;
    let (changed, checkpoint, ready) = (changed?, checkpoint?, ready?);
    let kinds = [&changed, &checkpoint, &ready].map(|event| event.event_type.clone());
    assert_eq!(
        kinds,
        [
            "geometry.changed",
            "geometry.checkpoint",
            "geometry.input_ready"
        ]
    );
    assert_eq!(changed.cursor, receipt);
    assert_eq!(
        checkpoint.cursor.seq.get(),
        changed
            .cursor
            .seq
            .get()
            .checked_add(1)
            .ok_or("seq overflow")?
    );
    assert_eq!(
        ready.cursor.seq.get(),
        checkpoint
            .cursor
            .seq
            .get()
            .checked_add(1)
            .ok_or("seq overflow")?
    );
    let GeometryEvent::Changed { previous, grid, .. } = changed.data else {
        return Err("first event is not the change".into());
    };
    assert_eq!(
        (previous, grid),
        (Geometry::new(80, 24)?, Geometry::new(100, 30)?)
    );
    let GeometryEvent::Checkpoint {
        grid,
        changed_seq,
        revision,
        ..
    } = checkpoint.data
    else {
        return Err("second event is not the checkpoint".into());
    };
    assert_eq!((grid, changed_seq), (Geometry::new(100, 30)?, receipt.seq));
    assert!(matches!(ready.data,
        GeometryEvent::InputReady { epoch: at, grid: ready_grid, revision: ready_revision, .. }
            if at == epoch && ready_grid == grid && ready_revision == revision));
    assert!(matches!(settled.resolutions.as_slice(),
        [resolution] if matches!(resolution.resolved, Resolved::Applied { grid: applied, .. } if applied == grid)));
    Ok(())
}

#[test]
fn a_resize_storm_sends_two_grids_and_supersedes_every_other_request() -> TestResult {
    // Given a transition to 100x30 in progress.
    let mut rig = Rig::new(screen(holder_target(1), 80, 24)?)?;
    let first = dispatched(rig.resize(&op(), (100, 30))??)?;
    rig.start(first)?;

    // When 100 more requests arrive before the holder acknowledges it.
    let mut ops = Vec::new();
    let mut superseded = Vec::new();
    for step in 0..100_u32 {
        let id = op();
        let requested = rig.resize(&id, (110_u32.saturating_add(step), 35))??;
        assert!(matches!(requested.outcome, Outcome::Queued { .. }));
        superseded.extend(requested.superseded);
        ops.push(id);
    }
    let settled = rig.acknowledged(GeometryEpoch::new(1))?;

    // Then every waiting request but the newest resolved superseded, naming its successor.
    assert_eq!(superseded.len(), 99);
    assert!(
        superseded
            .iter()
            .zip(ops.iter().zip(ops.iter().skip(1)))
            .all(|(gone, (replaced, by))| gone.op == *replaced && gone.by == *by)
    );
    // And completing the first checkpoints it without declaring input ready, starting the newest.
    assert!(matches!(
        settled.events.as_slice(),
        [GeometryEvent::Checkpoint { .. }]
    ));
    let next = settled.next.ok_or("the newest request did not start")?;
    assert_eq!(next.epoch(), GeometryEpoch::new(2));
    assert_eq!(
        rig.geometry.input_ready().err().map(|e| e.kind()),
        Some("not_ready")
    );
    let (resize, _) = rig.start(next)?;
    assert_eq!(resize.geometry, (209, 35));
    let last = rig.acknowledged(GeometryEpoch::new(2))?;
    assert!(matches!(
        last.events.as_slice(),
        [
            GeometryEvent::Checkpoint { .. },
            GeometryEvent::InputReady { .. }
        ]
    ));
    assert!(last.next.is_none());
    assert_eq!(rig.geometry.grid(), Geometry::new(209, 35)?);
    assert_eq!(rig.geometry.input_ready()?, GeometryEpoch::new(2));
    Ok(())
}

#[test]
fn requesting_the_current_or_in_flight_grid_sends_nothing() -> TestResult {
    // Given an 80x24 screen at epoch 0.
    let mut rig = Rig::new(screen(holder_target(1), 80, 24)?)?;

    // When the operator asks for 80x24.
    let same = rig.resize(&op(), (80, 24))??;

    // Then nothing starts and input stays ready at epoch 0.
    assert!(
        matches!(same.outcome, Outcome::Unchanged { epoch, .. } if epoch == GeometryEpoch::new(0))
    );
    assert_eq!(rig.geometry.input_ready()?, GeometryEpoch::new(0));

    // When a waiting 120x40 is followed by the grid already in flight.
    let first = dispatched(rig.resize(&op(), (100, 30))??)?;
    rig.start(first)?;
    let waiting = op();
    rig.resize(&waiting, (120, 40))??;
    let back = rig.resize(&op(), (100, 30))??;

    // Then the waiting request is superseded and completion starts nothing further.
    assert!(
        matches!(back.outcome, Outcome::Unchanged { epoch, .. } if epoch == GeometryEpoch::new(1))
    );
    assert_eq!(back.superseded.map(|gone| gone.op), Some(waiting));
    let settled = rig.acknowledged(GeometryEpoch::new(1))?;
    assert!(settled.next.is_none());
    assert_eq!(rig.geometry.grid(), Geometry::new(100, 30)?);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn any_storm_settles_on_the_newest_grid_with_at_most_two_dispatches(
        sizes in prop::collection::vec((2_u32..300, 1_u32..120), 1..24)
    ) {
        let mut rig = Rig::new(screen(holder_target(1), 80, 24).map_err(fail)?).map_err(fail)?;
        let first = dispatched(rig.resize(&op(), (100, 30)).map_err(fail)?.map_err(fail)?).map_err(fail)?;
        rig.start(first).map_err(fail)?;
        for size in &sizes {
            rig.resize(&op(), *size).map_err(fail)?.map_err(fail)?;
        }
        let mut epoch = GeometryEpoch::new(1);
        let mut dispatches = 1_u32;
        while let Some(next) = rig.acknowledged(epoch).map_err(fail)?.next {
            epoch = next.epoch();
            dispatches = dispatches.saturating_add(1);
            rig.start(next).map_err(fail)?;
        }
        let newest = sizes.last().copied().unwrap_or((100, 30));
        let expected = Geometry::new(u16::try_from(newest.0).map_err(fail)?, u16::try_from(newest.1).map_err(fail)?).map_err(fail)?;
        prop_assert_eq!(rig.geometry.grid(), expected);
        prop_assert!(dispatches <= 2);
        prop_assert_eq!(rig.geometry.input_ready().map_err(fail)?, epoch);
    }
}

fn fail(error: impl std::fmt::Debug) -> TestCaseError {
    TestCaseError::fail(format!("{error:?}"))
}
