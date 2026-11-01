//! Independent interact, geometry and observe leases against the real authority and commit path.

use std::error::Error;
use std::time::Duration;

use fixture::{Fixture, INTERACTOR, NOW, NoSections, OBSERVER, Root, TAKEOVER, takeover};
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::leases::{
    Acquire, ControlSlot, LeaseError, LeaseEvent, LeaseSlot, RevokeReason, Write, WriteKind,
};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, IdentityReservation, LeaseId, MachineId, RuntimeId, RuntimeIdentity,
    ServerBootId,
};
use ovis_terminal_store::{OpId, Store, StoreOptions};
use uuid::Uuid;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const BOUND: Duration = Duration::from_secs(5);

#[path = "../src/leases/qa/connections.rs"]
mod connections;
#[path = "../src/leases/qa/fixture.rs"]
pub mod fixture;
#[path = "../src/leases/qa/geometry.rs"]
mod geometry;
#[path = "../src/leases/qa/model.rs"]
mod model;
#[path = "../src/leases/qa/ownership.rs"]
mod ownership;
#[path = "../src/leases/qa/wire.rs"]
mod wire;

fn reserve(store: &Store, count: usize) -> TestResult<Vec<LeaseId>> {
    let ids = IdentityReservation::new(store);
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let mut reserved = Vec::with_capacity(count);
    for _ in 0..count {
        let op = OpId::new(Uuid::now_v7().to_string());
        reserved.push(runtime.block_on(ids.reserve::<LeaseId>(op))?);
    }
    Ok(reserved)
}

fn identity() -> TestResult<RuntimeIdentity> {
    Ok(RuntimeIdentity::new(
        HostIdentity::new(
            MachineId::new(Uuid::from_u128(24))?,
            HostBootId::try_from(1)?,
        ),
        RuntimeId::try_from(1)?,
        ServerBootId::try_from(1)?,
    ))
}

#[test]
fn happy() -> TestResult {
    // Given an operator holding interact and geometry under durably reserved lease ids.
    let root = Root::default();
    let store = Store::open(&root.0, StoreOptions::default())?;
    let mut ids = reserve(&store, 5)?.into_iter();
    let mut fresh = move || ids.next().ok_or("reserved lease ids exhausted");
    let core = Coordinator::new(store, identity()?, NoSections);
    let fixture = Fixture::new()?;
    let operator = fixture.client("operator", INTERACTOR)?;
    let phone = fixture.client("phone", OBSERVER)?;
    let browser = fixture.client("browser", OBSERVER)?;
    let bridge = fixture.client("bridge", TAKEOVER)?;
    let mut leases = fixture.leases()?;
    let input = leases.acquire(
        operator.caller(),
        Acquire::new(ControlSlot::Interact, fresh()?),
    )?;
    leases.acquire(
        operator.caller(),
        Acquire::new(ControlSlot::Geometry, fresh()?),
    )?;
    let before = leases.view();

    // When two observers attach.
    let watchers = [
        leases.observe(phone.caller(), fresh()?)?,
        leases.observe(browser.caller(), fresh()?)?,
    ];

    // Then the operator keeps both leases and the observers add only observe records.
    let after = leases.view();
    assert_eq!(after.interact, before.interact);
    assert_eq!(after.geometry, before.geometry);
    assert_eq!(
        after.observers,
        watchers
            .iter()
            .map(|granted| granted.lease.clone())
            .collect::<Vec<_>>()
    );
    for granted in &watchers {
        assert_eq!(granted.lease.slot, LeaseSlot::Observe);
        assert!(matches!(
            granted.transition.events.as_slice(),
            [LeaseEvent::Acquired { lease, displaced: None }] if lease == &granted.lease
        ));
    }
    let queued = leases.authorize_write(
        operator.caller(),
        Write::new(WriteKind::Input, Some(input.lease.lease_id)),
    )?;
    assert_eq!(queued.lease(), Some(&input.lease));

    // When the bridge takes over interact at the epoch it observed.
    let subscription = core.subscribe(None)?;
    let request = takeover(ControlSlot::Interact, input.lease.epoch, fresh()?)?;
    let taken = leases.take_over(bridge.caller(), &request)?;
    let commit = taken
        .transition
        .clone()
        .into_commit(OpId::new(Uuid::now_v7().to_string()))
        .ok_or("takeover produced no events")?;
    let receipt = core.commit(commit)?;

    // Then one durable unit publishes revoked before acquired, naming both owners.
    let revoked = subscription.delivery.recv_timeout(BOUND)?;
    let acquired = subscription.delivery.recv_timeout(BOUND)?;
    assert_eq!(revoked.event_type, "lease.revoked");
    assert_eq!(acquired.event_type, "lease.acquired");
    assert_eq!(
        revoked.cursor.seq.get().checked_add(1),
        Some(acquired.cursor.seq.get())
    );
    assert_eq!(acquired.cursor, receipt);
    assert!(matches!(
        &revoked.data,
        LeaseEvent::Revoked { lease, reason: RevokeReason::TakenOver, displaced_by: Some(by) }
            if lease == &input.lease && by == &taken.lease
    ));
    assert!(matches!(
        &acquired.data,
        LeaseEvent::Acquired { lease, displaced: Some(old) }
            if lease == &taken.lease && old == &input.lease
    ));
    assert_eq!(taken.lease.epoch, input.lease.epoch.checked_next()?);
    assert_eq!(taken.lease.owner, bridge.owner());
    assert_eq!(leases.view().geometry, before.geometry);
    assert_eq!(
        leases.recheck(&queued, NOW).err().map(|error| error.kind()),
        Some("lease_revoked")
    );
    println!("{}", serde_json::to_string(&[&revoked, &acquired])?);
    core.close()?;
    Ok(())
}

#[test]
fn failure() -> TestResult {
    // Given the bridge took interact over from the operator.
    let mut fixture = Fixture::new()?;
    let operator = fixture.client("operator", INTERACTOR)?;
    let rival = fixture.client("rival", INTERACTOR)?;
    let phone = fixture.client("phone", OBSERVER)?;
    let bridge = fixture.client("bridge", TAKEOVER)?;
    let relay = fixture.client("relay", TAKEOVER)?;
    let mut leases = fixture.leases()?;
    let first = leases.acquire(
        operator.caller(),
        Acquire::new(ControlSlot::Interact, fixture.fresh()?),
    )?;
    let stale_epoch = first.lease.epoch;
    let request = takeover(ControlSlot::Interact, stale_epoch, fixture.fresh()?)?;
    let current = leases.take_over(bridge.caller(), &request)?.lease;
    let before = leases.view();

    // When callers race it with stale, competing, observe-only and lease-less requests.
    let stale = takeover(ControlSlot::Interact, stale_epoch, fixture.fresh()?)?;
    let stale = leases.take_over(relay.caller(), &stale).err();
    let competing = leases
        .acquire(
            rival.caller(),
            Acquire::new(ControlSlot::Interact, fixture.fresh()?),
        )
        .err();
    let observe_input = leases
        .acquire(
            phone.caller(),
            Acquire::new(ControlSlot::Interact, fixture.fresh()?),
        )
        .err();
    let observe_grid = leases
        .acquire(
            phone.caller(),
            Acquire::new(ControlSlot::Geometry, fixture.fresh()?),
        )
        .err();
    let unpermitted = takeover(ControlSlot::Interact, current.epoch, fixture.fresh()?)?;
    let unpermitted = leases.take_over(rival.caller(), &unpermitted).err();
    let leaseless = leases
        .authorize_write(rival.caller(), Write::new(WriteKind::Input, None))
        .err();
    let borrowed = leases
        .authorize_write(
            rival.caller(),
            Write::new(WriteKind::Input, Some(current.lease_id)),
        )
        .err();

    // Then each is a typed rejection and the current owner is untouched.
    assert!(matches!(
        &stale,
        Some(LeaseError::EpochMismatch { expected, current: now })
            if *expected == stale_epoch && **now == current
    ));
    assert!(matches!(&competing, Some(LeaseError::Conflict { holder }) if **holder == current));
    let kinds = [
        stale,
        competing,
        observe_input,
        observe_grid,
        unpermitted,
        leaseless,
        borrowed,
    ]
    .map(|error| error.map(|error| error.kind()));
    assert_eq!(
        kinds,
        [
            Some("lease_epoch_mismatch"),
            Some("lease_conflict"),
            Some("forbidden"),
            Some("forbidden"),
            Some("forbidden"),
            Some("no_lease"),
            Some("no_lease"),
        ]
    );
    assert_eq!(leases.view(), before);
    Ok(())
}
