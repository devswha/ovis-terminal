//! Builds one commit unit from a pinned read view and a pure layout effect.

use ovis_terminal_protocol::schema::scalar::Revision;
use ovis_terminal_store::{CasWrite, Snapshot};

use super::LayoutError;
use super::edit::{Family, Fresh, NoOpReason, apply};
use super::model::Layout;
use super::persist::{read_layout, write_layout};
use super::records::{Change, LaunchOutcome, LayoutChanged, LayoutEvent, PaneBound};
use super::selection::clear_removed;
use super::service::EditRequest;
use crate::events::{EventError, ResourceId};
use crate::ops::{Completion, OperationError};
use crate::registry::{IdentityReservation, RuntimeId};

/// A planned unit: new layout row, ordered events and the method result.
#[derive(Debug)]
pub(super) struct Planned {
    pub change: Change,
    writes: Vec<CasWrite>,
    events: Vec<(LayoutEvent, Vec<ResourceId>)>,
}

impl Planned {
    const fn unchanged(layout: &Layout, no_op: Option<NoOpReason>) -> Self {
        Self {
            change: Change {
                revision: layout.revision,
                no_op,
                created: Vec::new(),
                removed: Vec::new(),
                changed: Vec::new(),
                released_terminals: Vec::new(),
                cleared_selections: Vec::new(),
                geometry: Vec::new(),
                launch: LaunchOutcome::NotRequested,
            },
            writes: Vec::new(),
            events: Vec::new(),
        }
    }

    pub(super) fn completion<D>(self) -> Result<Completion<D>, OperationError>
    where
        D: From<LayoutEvent>,
    {
        let result = serde_json::to_value(&self.change)?;
        Ok(self.attach(Completion::new(result)))
    }

    pub(super) fn pending<D>(self) -> Completion<D>
    where
        D: From<LayoutEvent>,
    {
        self.attach(Completion::pending())
    }

    fn attach<D: From<LayoutEvent>>(self, mut completion: Completion<D>) -> Completion<D> {
        for write in self.writes {
            completion = completion.write(write);
        }
        for (event, affected) in self.events {
            completion = completion.event(D::from(event), affected);
        }
        completion
    }
}

fn check_revision(layout: &Layout, expected: Option<Revision>) -> Result<(), LayoutError> {
    match expected {
        Some(expected) if expected != layout.revision => Err(LayoutError::RevisionMismatch {
            expected,
            current: layout.revision,
        }),
        _ => Ok(()),
    }
}

fn stage(snapshot: &Snapshot, needs: &[Family]) -> Result<(Fresh, Vec<CasWrite>), LayoutError> {
    let mut fresh = Fresh::default();
    let mut writes = Vec::new();
    for family in needs {
        match family {
            Family::Workspace => {
                let (id, staged) = IdentityReservation::stage(snapshot)?.into_parts();
                fresh.workspace = Some(id);
                writes.extend(staged);
            }
            Family::Tab => {
                let (id, staged) = IdentityReservation::stage(snapshot)?.into_parts();
                fresh.tab = Some(id);
                writes.extend(staged);
            }
            Family::Pane => {
                let (id, staged) = IdentityReservation::stage(snapshot)?.into_parts();
                fresh.pane = Some(id);
                writes.extend(staged);
            }
            Family::Split => {
                let (id, staged) = IdentityReservation::stage(snapshot)?.into_parts();
                fresh.split = Some(id);
                writes.extend(staged);
            }
        }
    }
    Ok((fresh, writes))
}

/// Stamps the revision this unit's final event will receive, after the operation event.
fn next_revision(snapshot: &Snapshot, events: usize) -> Result<Revision, LayoutError> {
    let events = u64::try_from(events).map_err(|_| EventError::LimitExceeded)?;
    snapshot
        .revision()
        .get()
        .checked_add(1)
        .and_then(|seq| seq.checked_add(events))
        .map(Revision::new)
        .ok_or(LayoutError::Event(EventError::LimitExceeded))
}

/// Applies a structural edit and returns the unit plus the scopes to authorize.
pub(super) fn plan_edit(
    snapshot: &Snapshot,
    runtime: RuntimeId,
    request: &EditRequest,
) -> Result<(Planned, Vec<crate::registry::WorkspaceId>), LayoutError> {
    let (mut layout, version) = read_layout(snapshot, runtime)?;
    check_revision(&layout, request.expect_revision)?;
    let (fresh, mut writes) = stage(snapshot, request.edit.needs())?;
    let mut effect = apply(&mut layout, &request.edit, fresh)?;
    let scopes = std::mem::take(&mut effect.scopes);
    if effect.no_op.is_some() {
        return Ok((Planned::unchanged(&layout, effect.no_op), scopes));
    }
    let mut events = Vec::new();
    let mut cleared = Vec::new();
    for (selection, previous) in clear_removed(&mut layout, &effect.removed) {
        cleared.push(selection.client.clone());
        events.push((LayoutEvent::Selection(selection), previous));
    }
    let affected = effect.affected();
    events.push((
        LayoutEvent::Changed(LayoutChanged {
            revision: Revision::new(0),
            created: effect.created.clone(),
            removed: effect.removed.clone(),
            changed: effect.changed.clone(),
        }),
        affected,
    ));
    layout.revision = next_revision(snapshot, events.len())?;
    writes.push(write_layout(runtime, version, &layout)?);
    let mut geometry = Vec::new();
    for tab in &effect.tabs {
        if let Ok(at) = layout.tab_at(*tab) {
            geometry.push(layout.tab(at)?.geometry()?);
        }
    }
    let change = Change {
        revision: layout.revision,
        no_op: None,
        created: effect.created,
        removed: effect.removed,
        changed: effect.changed,
        released_terminals: effect.released,
        cleared_selections: cleared,
        geometry,
        launch: LaunchOutcome::NotRequested,
    };
    Ok((
        Planned {
            change,
            writes,
            events,
        },
        scopes,
    ))
}

/// Binds a launched terminal to its created pane and records the launch outcome.
pub(super) fn plan_bind(
    snapshot: &Snapshot,
    runtime: RuntimeId,
    (mut change, outcome): (Change, LaunchOutcome),
) -> Result<Planned, LayoutError> {
    let pane = change.created.iter().find_map(|id| match id {
        ResourceId::Pane(pane) => Some(*pane),
        _ => None,
    });
    let launched = match &outcome {
        LaunchOutcome::Launched { terminal, .. } => Some(*terminal),
        LaunchOutcome::NotRequested | LaunchOutcome::Rejected { .. } => None,
    };
    change.launch = outcome;
    let (Some(pane), Some(terminal)) = (pane, launched) else {
        return Ok(Planned {
            change,
            writes: Vec::new(),
            events: Vec::new(),
        });
    };
    let (mut layout, version) = read_layout(snapshot, runtime)?;
    let Ok(at) = layout.pane_at(pane) else {
        change.released_terminals.push(terminal);
        return Ok(Planned {
            change,
            writes: Vec::new(),
            events: Vec::new(),
        });
    };
    let record = layout
        .tab_mut(at)?
        .panes
        .iter_mut()
        .find(|record| record.id == pane)
        .ok_or(LayoutError::Event(EventError::InvalidRecord))?;
    record.terminal = Some(terminal);
    layout.revision = next_revision(snapshot, 1)?;
    change.revision = layout.revision;
    change.changed.push(ResourceId::Pane(pane));
    let event = LayoutEvent::Bound(PaneBound {
        revision: layout.revision,
        pane,
        terminal,
    });
    Ok(Planned {
        change,
        writes: vec![write_layout(runtime, version, &layout)?],
        events: vec![(
            event,
            vec![ResourceId::Pane(pane), ResourceId::Terminal(terminal)],
        )],
    })
}

/// Commits a selection change for exactly one client.
pub(super) fn plan_select(
    snapshot: &Snapshot,
    (runtime, machine): (RuntimeId, crate::registry::MachineId),
    request: &super::SelectRequest,
) -> Result<Planned, LayoutError> {
    let (mut layout, version) = read_layout(snapshot, runtime)?;
    check_revision(&layout, request.expect_revision)?;
    let Some(selection) = super::selection::select(&mut layout, request)? else {
        return Ok(Planned::unchanged(
            &layout,
            Some(NoOpReason::AlreadyApplied),
        ));
    };
    let mut affected = selection.affected();
    if affected.is_empty() {
        affected.push(ResourceId::Machine(machine));
    }
    layout.revision = next_revision(snapshot, 1)?;
    let mut change = Planned::unchanged(&layout, None).change;
    change.changed.clone_from(&affected);
    Ok(Planned {
        change,
        writes: vec![write_layout(runtime, version, &layout)?],
        events: vec![(LayoutEvent::Selection(selection), affected)],
    })
}
