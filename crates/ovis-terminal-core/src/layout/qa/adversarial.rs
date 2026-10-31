//! Malformed input, replayed operations and atomic session snapshots.

use ovis_terminal_core::events::{Coordinator, Section};
use ovis_terminal_core::layout::{
    Call, Edit, EditRequest, LayoutError, LayoutEvent, SessionProducer, Target,
};
use ovis_terminal_core::registry::PaneId;

use super::{Fixture, TestResult, op, workspace_with_split};

/// Unknown fields, numeric counters and unknown targets fail typed with no effect.
///
/// # Errors
///
/// Returns store, authority, operation or layout failures of the fixture.
///
/// # Panics
///
/// Panics when an observed layout fact violates the acceptance contract.
pub fn malformed_requests_are_rejected_before_any_effect() -> TestResult {
    // Given: a layout with one split tab.
    let fixture = Fixture::new()?;
    let (first, _) = workspace_with_split(&fixture)?;
    let pane = first.pane.ok_or("pane")?;
    let before = fixture.layout()?;

    // When: a misnamed target field, a numeric revision and an unknown pane are sent.
    let misnamed = format!(
        r#"{{"edit":{{"op":"split","pane_id":"{pane}","axis":"horizontal","ratio_permille":500}}}}"#
    );
    let numeric = format!(
        r#"{{"expect_revision":{},"edit":{{"op":"zoom","pane":"{pane}","zoomed":true}}}}"#,
        before.revision.get()
    );
    let unknown = fixture.attempt(&EditRequest::new(Edit::Close {
        target: Target::Pane(PaneId::try_from(u64::MAX)?),
        scope: ovis_terminal_core::layout::CloseScope::Target,
    }))?;

    // Then: each is a typed rejection and the layout, focus and selections are untouched.
    for bytes in [misnamed.as_bytes(), numeric.as_bytes()] {
        let decoded = EditRequest::decode(bytes);
        assert_eq!(decoded.err().map(|e| e.kind()), Some("invalid_params"));
    }
    assert!(matches!(&unknown, Err(LayoutError::NotFound { .. })));
    let target = unknown.err().and_then(|error| error.data().target);
    assert_eq!(
        serde_json::to_value(target)?,
        serde_json::Value::String(PaneId::try_from(u64::MAX)?.to_string())
    );
    assert_eq!(fixture.layout()?, before);
    Ok(())
}

/// The same operation converges on one result; a reused identity is refused.
///
/// # Errors
///
/// Returns store, authority, operation or layout failures of the fixture.
///
/// # Panics
///
/// Panics when an observed layout fact violates the acceptance contract.
pub fn replay_converges_and_reuse_is_refused() -> TestResult {
    // Given: one committed split under a client-minted operation identity.
    let fixture = Fixture::new()?;
    let (first, _) = workspace_with_split(&fixture)?;
    let pane = first.pane.ok_or("pane")?;
    let identity = op();
    let zoom = || EditRequest::new(Edit::Zoom { pane, zoomed: true });
    let call = || Call::new(&fixture.session, identity.clone());
    let applied = fixture.layouts()?.edit(call(), &zoom())?;
    let after_first = fixture.layout()?;

    // When: the identical request is retried, then the identity is reused for other work.
    let replayed = fixture.layouts()?.edit(call(), &zoom())?;
    let reused = fixture.layouts()?.edit(
        call(),
        &EditRequest::new(Edit::Zoom {
            pane,
            zoomed: false,
        }),
    );

    // Then: the retry returns the recorded result without a second commit.
    assert_eq!(replayed, applied);
    assert_eq!(fixture.layout()?, after_first);
    assert_eq!(reused.err().map(|e| e.kind()), Some("op_reused"));
    assert_eq!(fixture.layout()?, after_first);

    // Then: a request that matches current state is an explicit no-op, not a new revision.
    let again = fixture.edit(Edit::Zoom { pane, zoomed: true })?;
    assert_eq!(
        again.no_op,
        Some(ovis_terminal_core::layout::NoOpReason::AlreadyApplied)
    );
    assert_eq!(again.revision, after_first.revision);
    Ok(())
}

/// The session snapshot carries complete layout and terminal sections at one cursor.
///
/// # Errors
///
/// Returns store, authority, operation or layout failures of the fixture.
///
/// # Panics
///
/// Panics when an observed layout fact violates the acceptance contract.
pub fn session_snapshot_holds_layout_and_terminals_at_one_revision() -> TestResult {
    // Given: committed layout state from the layout service.
    let mut fixture = Fixture::new()?;
    let (first, _) = workspace_with_split(&fixture)?;
    fixture.select("phone", (first.workspace, first.tab, first.pane))?;
    let expected = fixture.layout()?;
    let identity = fixture.identity;

    // When: the same store is captured by the composed session producer.
    let store = fixture.reopen_store()?;
    let session: Coordinator<LayoutEvent, SessionProducer> =
        Coordinator::new(store, identity, SessionProducer::new(identity.runtime_id()));
    let snapshot = session.read_snapshot()?;
    session.close()?;

    // Then: layout and terminal sections are complete and pinned at the layout revision.
    assert_eq!(
        snapshot.sections.sections.layout,
        Section::Complete(expected.clone())
    );
    assert_eq!(
        snapshot.sections.sections.terminals,
        Section::Complete(Vec::new())
    );
    assert_eq!(snapshot.cursor.seq.get(), expected.revision.get());
    Ok(())
}
