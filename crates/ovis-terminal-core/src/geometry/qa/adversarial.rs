//! Typed refusals beyond the QA failure case: non-owners, malformed grids, released owners
//! and focus claims.

use ovis_terminal_core::geometry::{Fence, GeometryEpoch, Outcome, Resolved};
use ovis_terminal_vt::Geometry;

use super::TestResult;
use super::fixture::{NOW, OBSERVER, WRITER};
use super::rig::{Rig, dispatched, holder_target, op, params, screen};

#[test]
fn non_owners_and_malformed_grids_are_refused_without_a_change() -> TestResult {
    // Given the operator owns geometry and a rival holds none.
    let mut rig = Rig::new(screen(holder_target(1), 80, 24)?)?;
    let rival = rig.fixture.client("rival", WRITER)?;

    // When the rival presents the operator's lease, and the operator asks for impossible grids.
    let foreign = rig.leases.authorize_resize(rival.caller(), rig.grid).err();
    let zero = rig.resize(&op(), (0, 30))?.err();
    let narrow = rig.resize(&op(), (1, 30))?.err();
    let flat = rig.resize(&op(), (120, 0))?.err();
    let overflow = params(&op(), rig.grid, (70_000, 30));

    // Then each is a typed refusal and the grid, epoch and readiness are untouched.
    let kinds = [
        foreign.map(|error| error.kind()),
        zero.map(|error| error.kind()),
        narrow.map(|error| error.kind()),
        flat.map(|error| error.kind()),
    ];
    assert_eq!(
        kinds,
        [
            Some("not_geometry_owner"),
            Some("invalid_geometry"),
            Some("invalid_geometry"),
            Some("invalid_geometry"),
        ]
    );
    assert!(
        overflow.is_err(),
        "70000 columns must not decode: {overflow:?}"
    );
    assert_eq!(rig.geometry.grid(), Geometry::new(80, 24)?);
    assert_eq!(rig.geometry.input_ready()?, GeometryEpoch::new(0));
    Ok(())
}

#[test]
fn a_released_owners_waiting_resize_is_refused_and_the_grid_stays() -> TestResult {
    // Given a transition in progress and the operator's next request waiting.
    let mut rig = Rig::new(screen(holder_target(1), 80, 24)?)?;
    let first = dispatched(rig.resize(&op(), (100, 30))??)?;
    rig.start(first)?;
    let waiting = op();
    assert!(matches!(
        rig.resize(&waiting, (120, 40))??.outcome,
        Outcome::Queued { .. }
    ));

    // When the operator releases geometry.
    rig.leases.release(rig.operator.caller(), rig.grid)?;
    let refused = rig.geometry.lease_changed(Fence::new(&rig.leases, NOW));

    // Then the waiting request is refused as revoked and only the sent grid completes.
    let refused = refused.ok_or("the waiting request was not refused")?;
    assert_eq!(refused.op, waiting);
    assert!(
        matches!(&refused.resolved, Resolved::Refused(error) if error.kind() == "lease_revoked")
    );
    let settled = rig.acknowledged(GeometryEpoch::new(1))?;
    assert!(settled.next.is_none());
    assert_eq!(rig.geometry.grid(), Geometry::new(100, 30)?);
    assert!(rig.leases.view().geometry.is_none());
    Ok(())
}

#[test]
fn focus_records_the_client_and_never_moves_geometry_or_input() -> TestResult {
    // Given the operator owns input and geometry and a phone observes.
    let mut rig = Rig::new(screen(holder_target(1), 80, 24)?)?;
    let phone = rig.fixture.client("phone", OBSERVER)?;
    let watch = rig.fixture.fresh()?;
    let observed = rig.leases.observe(phone.caller(), watch)?;
    let before = rig.leases.view();

    // When the phone reports focus, and then claims the operator's lease.
    let phone_owner = observed.lease.owner.clone();
    let focused = rig.geometry.focus(&before, (&phone_owner, watch))?;
    let borrowed = rig.geometry.focus(&before, (&phone_owner, rig.grid)).err();

    // Then focus is recorded for the phone only and both control slots stay with the operator.
    assert_eq!(focused.lease, observed.lease);
    assert_eq!(focused.geometry_owner, before.geometry);
    assert_eq!(rig.geometry.focused(), Some(watch));
    assert_eq!(borrowed.map(|error| error.kind()), Some("no_lease"));
    assert_eq!(rig.leases.view(), before);
    assert_eq!(rig.geometry.input_ready()?, GeometryEpoch::new(0));
    Ok(())
}
