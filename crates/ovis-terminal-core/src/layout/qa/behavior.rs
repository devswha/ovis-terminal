//! The named happy and failure acceptance cases.

use std::time::Duration;

use ovis_terminal_core::events::ResourceId;
use ovis_terminal_core::layout::{
    Axis, CloseScope, Edit, EditRequest, GeometryReason, LayoutError, LayoutEvent, Target,
};
use ovis_terminal_core::ops::{Context, Operations};
use ovis_terminal_protocol::schema::methods::server::OpState;
use ovis_terminal_protocol::schema::scalar::{OpenEnum, Revision, TimestampMs};

use super::{Fixture, TestResult, created, op, tab_shape, workspace_with_split};

/// Move and swap keep pane and split identities and both clients' selections.
///
/// # Errors
///
/// Returns store, authority, operation or layout failures of the fixture.
///
/// # Panics
///
/// Panics when an observed layout fact violates the acceptance contract.
pub fn happy() -> TestResult {
    // Given: a subscribed client, one tab split twice and two independent selections.
    let fixture = Fixture::new()?;
    let subscription = fixture.core()?.subscribe(None)?;
    let (first, split) = workspace_with_split(&fixture)?;
    let (workspace, tab) = (first.workspace.ok_or("w")?, first.tab.ok_or("t")?);
    let (left, right) = (first.pane.ok_or("p1")?, split.pane.ok_or("p2")?);
    let inner = created(&fixture.edit(Edit::Split {
        pane: right,
        axis: Axis::Vertical,
        ratio_permille: 250,
    })?);
    let bottom = inner.pane.ok_or("p3")?;
    fixture.select("phone", (Some(workspace), Some(tab), Some(left)))?;
    fixture.select("desk", (Some(workspace), Some(tab), Some(bottom)))?;
    let before = fixture.layout()?;
    let rect_of = |layout: &ovis_terminal_core::layout::Layout, pane| {
        let tab = layout
            .workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .find(|t| t.id == tab)?;
        let geometry = tab.geometry().ok()?;
        geometry
            .panes
            .into_iter()
            .find(|g| g.pane == pane)
            .map(|g| g.rect)
    };

    // When: two panes swap and the whole tab moves to another workspace.
    let swapped = fixture.edit(Edit::Swap {
        first: left,
        second: bottom,
    })?;
    let other = created(&fixture.edit(Edit::CreateWorkspace {
        name: "other".into(),
        group_with: None,
    })?);
    let destination = other.workspace.ok_or("w2")?;
    let moved = fixture.edit(Edit::MoveTab {
        tab,
        workspace: destination,
        index: 1,
    })?;
    let after = fixture.layout()?;

    // Then: identities, tree nodes and every client's selection survive unchanged.
    assert_eq!(tab_shape(&after, tab), tab_shape(&before, tab));
    assert_eq!(after.selections, before.selections);
    assert_eq!(after.selections.len(), 2);
    assert_eq!(rect_of(&after, left), rect_of(&before, bottom));
    assert_eq!(rect_of(&after, bottom), rect_of(&before, left));
    assert!(swapped.created.is_empty() && swapped.removed.is_empty());
    assert!(swapped.changed.contains(&ResourceId::Pane(left)));
    assert!(swapped.changed.contains(&ResourceId::Pane(bottom)));
    let host = after
        .workspaces
        .iter()
        .find(|w| w.id == destination)
        .ok_or("moved")?;
    assert_eq!(host.tabs.get(1).map(|t| t.id), Some(tab));
    assert!(
        after
            .workspaces
            .iter()
            .any(|w| w.id == workspace && w.tabs.is_empty())
    );
    assert!(moved.revision > swapped.revision && swapped.revision > before.revision);
    assert_eq!(after.revision, moved.revision);

    // Then: every committed change arrived as a revisioned event naming its resources.
    let mut layout_events = Vec::new();
    while let Ok(event) = subscription.delivery.recv_timeout(Duration::from_millis(0)) {
        if let LayoutEvent::Changed(changed) = &event.data {
            assert_eq!(changed.revision.get(), event.cursor.seq.get());
            layout_events.push((changed.revision, event.affected.clone()));
        }
    }
    let swap_event = layout_events
        .iter()
        .find(|(revision, _)| *revision == swapped.revision);
    let swap_affected = &swap_event.ok_or("swap event")?.1;
    assert!(swap_affected.contains(&ResourceId::Pane(left)));
    assert!(swap_affected.contains(&ResourceId::Tab(tab)));
    let move_event = layout_events
        .iter()
        .find(|(revision, _)| *revision == moved.revision);
    let move_affected = &move_event.ok_or("move event")?.1;
    assert!(move_affected.contains(&ResourceId::Workspace(destination)));
    assert!(move_affected.contains(&ResourceId::Workspace(workspace)));
    assert_eq!(layout_events.len(), 6);

    #[cfg(unix)]
    super::launch::failed_launch_returns_created_ids(&fixture, destination)?;
    Ok(())
}

/// Out-of-range geometry and group closes are typed rejections that change nothing.
///
/// # Errors
///
/// Returns store, authority, operation or layout failures of the fixture.
///
/// # Panics
///
/// Panics when an observed layout fact violates the acceptance contract.
pub fn failure() -> TestResult {
    // Given: a split tab and a workspace linked into its group.
    let fixture = Fixture::new()?;
    let (first, split) = workspace_with_split(&fixture)?;
    let (workspace, node) = (first.workspace.ok_or("w")?, split.split.ok_or("s")?);
    let linked = created(&fixture.edit(Edit::CreateWorkspace {
        name: "linked".into(),
        group_with: Some(workspace),
    })?);
    fixture.select("phone", (Some(workspace), first.tab, split.pane))?;
    let before = fixture.layout()?;

    // When: a ratio outside the supported range is requested.
    let ratio_op = op();
    let rejected = fixture.layouts()?.edit(
        ovis_terminal_core::layout::Call::new(&fixture.session, ratio_op.clone()),
        &EditRequest::new(Edit::SetRatio {
            split: node,
            ratio_permille: 950,
        }),
    );

    // Then: it is invalid_geometry with the exact range, nothing was clamped, and the
    // failed operation is inspectable.
    let Err(LayoutError::InvalidGeometry(rejection)) = &rejected else {
        return Err(format!("expected invalid_geometry, got {rejected:?}").into());
    };
    assert_eq!(
        rejected.as_ref().err().map(LayoutError::kind),
        Some("invalid_geometry")
    );
    assert_eq!(
        (
            rejection.reason,
            rejection.requested,
            rejection.min,
            rejection.max
        ),
        (GeometryReason::OutOfRange, 950, 100, 900)
    );
    assert_eq!(fixture.layout()?, before);
    let context = Context::new(
        fixture.session.principal().clone(),
        fixture.identity,
        now()?,
    );
    let record = Operations::new(fixture.core()?, context).get(&ratio_op)?;
    assert_eq!(record.state, OpenEnum::Known(OpState::Failed));
    assert_eq!(
        record.error.map(|e| e.kind.token().to_owned()).as_deref(),
        Some("invalid_geometry")
    );

    // Then: tab-relative resizes are exact or rejected with their range and step.
    let unrepresentable = fixture.attempt(&EditRequest::new(Edit::Resize {
        split: node,
        delta_ppm: 1,
    }))?;
    assert!(
        matches!(&unrepresentable, Err(LayoutError::InvalidGeometry(r))
        if r.reason == GeometryReason::Unrepresentable && r.step == 1000)
    );
    let beyond = fixture.attempt(&EditRequest::new(Edit::Resize {
        split: node,
        delta_ppm: 450_000,
    }))?;
    assert!(matches!(&beyond, Err(LayoutError::InvalidGeometry(r))
        if r.reason == GeometryReason::OutOfRange && r.max == 400_000));
    assert_eq!(fixture.layout()?, before);

    // When: the grouped workspace is closed with only target scope.
    let close = |scope| Edit::Close {
        target: Target::Workspace(workspace),
        scope,
    };
    let refused = fixture.attempt(&EditRequest::new(close(CloseScope::Target)))?;

    // Then: group_scope_required names every affected identity and nothing closes.
    let Err(LayoutError::GroupScopeRequired { affected }) = &refused else {
        return Err(format!("expected group_scope_required, got {refused:?}").into());
    };
    assert_eq!(
        refused.as_ref().err().map(LayoutError::kind),
        Some("group_scope_required")
    );
    for id in [
        ResourceId::Workspace(workspace),
        ResourceId::Workspace(linked.workspace.ok_or("linked")?),
        ResourceId::Pane(split.pane.ok_or("p2")?),
        ResourceId::Split(node),
    ] {
        assert!(affected.contains(&id), "{id:?} is named");
    }
    assert_eq!(fixture.layout()?, before);

    // When: the same close carries explicit group scope and a stale revision guard.
    let mut stale = EditRequest::new(close(CloseScope::Group));
    stale.expect_revision = Some(Revision::new(before.revision.get().saturating_sub(1)));
    let conflict = fixture.attempt(&stale)?;
    assert!(
        matches!(conflict, Err(LayoutError::RevisionMismatch { current, .. }) if current == before.revision)
    );
    let mut guarded = EditRequest::new(close(CloseScope::Group));
    guarded.expect_revision = Some(before.revision);
    let closed = fixture.attempt(&guarded)??;

    // Then: both linked workspaces close and the removed selection is cleared, not retargeted.
    let after = fixture.layout()?;
    assert_eq!(after.workspaces, Vec::new());
    assert_eq!(closed.cleared_selections.len(), 1);
    let phone = after.selections.first().ok_or("phone")?;
    assert_eq!((phone.workspace, phone.tab, phone.pane), (None, None, None));
    Ok(())
}

fn now() -> Result<TimestampMs, Box<dyn std::error::Error>> {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    Ok(TimestampMs::new(u64::try_from(millis)?))
}
