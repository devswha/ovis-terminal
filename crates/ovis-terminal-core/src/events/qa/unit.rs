//! Atomic multi-event units keep all resources and their replay together.

use super::*;

#[test]
fn unit_failure_rolls_back_every_write_and_event() -> TestResult {
    // Given: one existing terminal, and a subscribed observer.
    let fixture = Fixture::new();
    let core = Coordinator::new(
        fixture.open(StoreOptions::default())?,
        identity(1)?,
        Producer,
    );
    core.commit(change("before", 0)?)?;
    let observer = core.subscribe(None)?;
    let before = observer.snapshot.ok_or("snapshot")?;

    // When: a unit writes its operation row before encountering a terminal CAS failure.
    let commit = change("after", 0)?
        .write(CasWrite::new(
            ResourceKey::new("operation", "failed-unit"),
            Version::ABSENT,
            Some(Payload::from("not committed")),
        ))
        .event(
            Changed {
                value: "receipt".into(),
            },
            vec![ResourceId::Terminal(TerminalId::try_from(1)?)],
        );
    let rejected = core.commit(commit).expect_err("stale terminal version");

    // Then: neither state nor events advance.
    assert_eq!(rejected.kind(), "conflict");
    assert_eq!(core.read_snapshot()?.cursor, before.cursor);
    assert_eq!(core.read_snapshot()?.sections, before.sections);
    assert!(matches!(
        observer.delivery.try_recv(),
        Err(EventError::TryReceive(mpsc::TryRecvError::Empty))
    ));
    core.close()?;
    let store = fixture.open(StoreOptions::default())?;
    assert!(matches!(
        store
            .snapshot()?
            .resource(&ResourceKey::new("operation", "failed-unit"))?,
        ResourceLookup::Absent
    ));
    store.close()?;
    Ok(())
}

#[test]
fn unit_builder_failure_publishes_nothing() -> TestResult {
    // Given: an empty coordinator and an installed observer.
    let fixture = Fixture::new();
    let core = Coordinator::<Changed, _>::new(
        fixture.open(StoreOptions::default())?,
        identity(1)?,
        Producer,
    );
    let observer = core.subscribe(None)?;
    let before = observer.snapshot.ok_or("snapshot")?.cursor;

    // When: constructing the unit fails before submission.
    let result: Result<_, EventError> =
        core.commit_unit(|_snapshot| Err(EventError::InvalidRecord));

    // Then: there is no mutation and no externally visible event.
    assert_eq!(result.expect_err("rejected unit").kind(), "invalid_event");
    assert_eq!(core.read_snapshot()?.cursor, before);
    assert!(matches!(
        observer.delivery.try_recv(),
        Err(EventError::TryReceive(mpsc::TryRecvError::Empty))
    ));
    core.close()?;
    Ok(())
}
