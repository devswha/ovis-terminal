//! Who may resize and focus: only a geometry-lease authorization resizes, focus is bound to
//! the authenticated session, and the holder message needs this transition's own receipt.

use ovis_terminal_core::events::EventError;
use ovis_terminal_core::geometry::{GeometryAuthorization, GeometryController, GeometryEpoch};
use ovis_terminal_core::leases::{Acquire, ControlSlot, Write, WriteKind};
use ovis_terminal_core::registry::TerminalId;
use ovis_terminal_vt::Geometry;

use super::TestResult;
use super::fixture::WRITER;
use super::lease_fixture::{Fixture as LeaseFixture, INTERACTOR, OBSERVER};
use super::rig::{Rig, dispatched, holder_target, op, screen, store_op};

#[test]
fn a_write_authorization_never_becomes_a_resize() -> TestResult {
    // Given the operator keeps geometry and a typist holds only the interact lease.
    let mut rig = Rig::new(screen(holder_target(1), 80, 24)?)?;
    let subscription = rig.core()?.subscribe(None)?;
    rig.leases.release(rig.operator.caller(), rig.input)?;
    let typist = rig.fixture.client("typist", WRITER)?;
    let typed = rig.fixture.interact(&mut rig.leases, &typist)?;
    let write = rig
        .leases
        .authorize_write(typist.caller(), Write::new(WriteKind::Input, Some(typed)));

    // When the typist asks for a resize under its interact lease or the operator's grid lease.
    // (Handing `request` the write authorization itself does not compile: it takes only a
    // `GeometryAuthorization`, which only `authorize_resize` produces.)
    let with_input = GeometryAuthorization::admit(&rig.leases, typist.caller(), typed).err();
    let with_grid = GeometryAuthorization::admit(&rig.leases, typist.caller(), rig.grid).err();

    // Then both are not_geometry_owner, nothing was dispatched or committed, the grid stays.
    assert!(write.is_ok(), "the typist may write: {write:?}");
    let kinds = [with_input, with_grid].map(|error| error.map(|error| error.kind()));
    assert_eq!(kinds, [Some("not_geometry_owner"); 2]);
    assert!(subscription.delivery.try_recv().is_err());
    assert_eq!(rig.geometry.grid(), Geometry::new(80, 24)?);
    assert_eq!(rig.geometry.input_ready()?, GeometryEpoch::new(0));
    Ok(())
}

#[test]
fn focus_is_bound_to_the_authenticated_session_and_moves_nothing() -> TestResult {
    // Given the operator owns input and geometry and a phone observes.
    let mut fixture = LeaseFixture::new()?;
    let operator = fixture.client("operator", INTERACTOR)?;
    let phone = fixture.client("phone", OBSERVER)?;
    let mut leases = fixture.leases()?;
    let input = Acquire::new(ControlSlot::Interact, fixture.fresh()?);
    let input = leases.acquire(operator.caller(), input)?.lease;
    let grid = Acquire::new(ControlSlot::Geometry, fixture.fresh()?);
    leases.acquire(operator.caller(), grid)?;
    let watch = fixture.fresh()?;
    let observed = leases.observe(phone.caller(), watch)?.lease;
    let view = leases.view();
    let terminal = TerminalId::try_from(7)?;
    let mut geometry = GeometryController::new(
        terminal,
        screen(holder_target(1), 80, 24)?,
        GeometryEpoch::new(0),
    )?;

    // When the phone reports focus, then claims the operator's lease and client label.
    let focused = geometry.focus(&view, (&phone.session, &phone.client, watch))?;
    let impersonated = geometry
        .focus(&view, (&phone.session, &operator.client, input.lease_id))
        .err();

    // Then only the phone's own lease is focused and both control slots stay with the operator.
    assert_eq!(focused.lease, observed);
    assert_eq!(focused.geometry_owner, view.geometry);
    assert_eq!(geometry.focused(), Some(watch));
    assert_eq!(impersonated.map(|error| error.kind()), Some("no_lease"));
    assert_eq!(leases.view(), view);
    assert_eq!(geometry.input_ready()?, GeometryEpoch::new(0));
    Ok(())
}

#[test]
fn a_receipt_from_an_earlier_commit_does_not_release_the_holder_message() -> TestResult {
    // Given epoch 1 completed and a transition to 120x40 dispatched.
    let mut rig = Rig::new(screen(holder_target(1), 80, 24)?)?;
    let first = dispatched(rig.resize(&op(), (100, 30))??)?;
    let (_, earlier) = rig.start(first)?;
    rig.acknowledged(GeometryEpoch::new(1))?;
    let second = dispatched(rig.resize(&op(), (120, 40))??)?;

    // When its commit path reports epoch 1's receipt instead of its own.
    let core = rig.core()?;
    let committed = second.commit(store_op(), |unit| {
        core.commit(unit)?;
        Ok::<_, EventError>(earlier)
    })?;
    let foreign = rig.geometry.apply(&committed).err();

    // Then the receipt is refused, the VT keeps 100x30 and input stays not ready.
    assert_eq!(foreign.map(|error| error.kind()), Some("invalid_params"));
    assert_eq!(rig.screen.first_paint()?.geometry, Geometry::new(100, 30)?);
    let gate = rig.geometry.input_ready().err().map(|error| error.kind());
    assert_eq!(gate, Some("not_ready"));
    Ok(())
}
