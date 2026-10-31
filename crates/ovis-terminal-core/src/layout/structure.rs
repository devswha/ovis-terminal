//! Create, split, move and close; nothing beyond the named scope is touched.

use super::LayoutError;
use super::edit::{Effect, Fresh, missing};
use super::model::{
    Axis, Child, Layout, MAX_PANES, MAX_TABS, MAX_WORKSPACES, Pane, Ratio, Split, Tab, Workspace,
    label,
};
use crate::events::ResourceId;
use crate::registry::{PaneId, TabId, WorkspaceId};

fn new_tab(id: TabId, pane: Pane) -> Tab {
    Tab {
        id,
        root: Some(Child::Pane(pane.id)),
        splits: Vec::new(),
        panes: vec![pane],
        zoomed: None,
    }
}

pub(super) fn create_workspace(
    layout: &mut Layout,
    (name, group_with): (&str, Option<WorkspaceId>),
    fresh: Fresh,
) -> Result<Effect, LayoutError> {
    let name = label(name, "edit.name")?;
    if layout.workspaces.len() >= MAX_WORKSPACES {
        return Err(LayoutError::LimitExceeded);
    }
    let (id, tab, pane) = (
        fresh.workspace.ok_or_else(missing)?,
        fresh.tab.ok_or_else(missing)?,
        fresh.pane.ok_or_else(missing)?,
    );
    let mut effect = Effect::default();
    let group = match group_with {
        Some(anchor) => {
            let index = layout.workspace_at(anchor)?;
            let linked = layout.workspaces.get_mut(index).ok_or_else(missing)?;
            let group = *linked.group.get_or_insert(anchor);
            effect.changed.push(ResourceId::Workspace(anchor));
            Some(group)
        }
        None => None,
    };
    layout.workspaces.push(Workspace {
        id,
        name,
        group,
        tabs: vec![new_tab(
            tab,
            Pane {
                id: pane,
                terminal: None,
            },
        )],
    });
    effect.created = vec![
        ResourceId::Workspace(id),
        ResourceId::Tab(tab),
        ResourceId::Pane(pane),
    ];
    effect.tabs.push(tab);
    Ok(effect)
}

pub(super) fn create_tab(
    layout: &mut Layout,
    workspace: WorkspaceId,
    fresh: Fresh,
) -> Result<Effect, LayoutError> {
    let index = layout.workspace_at(workspace)?;
    let (tab, pane) = (
        fresh.tab.ok_or_else(missing)?,
        fresh.pane.ok_or_else(missing)?,
    );
    let target = layout.workspaces.get_mut(index).ok_or_else(missing)?;
    if target.tabs.len() >= MAX_TABS {
        return Err(LayoutError::LimitExceeded);
    }
    target.tabs.push(new_tab(
        tab,
        Pane {
            id: pane,
            terminal: None,
        },
    ));
    Ok(Effect {
        created: vec![ResourceId::Tab(tab), ResourceId::Pane(pane)],
        changed: vec![ResourceId::Workspace(workspace)],
        scopes: vec![workspace],
        tabs: vec![tab],
        ..Effect::default()
    })
}

pub(super) fn split(
    layout: &mut Layout,
    (pane, axis, permille): (PaneId, Axis, u16),
    fresh: Fresh,
) -> Result<Effect, LayoutError> {
    let at = layout.pane_at(pane)?;
    let ratio = Ratio::new(permille)?;
    let (split, created) = (
        fresh.split.ok_or_else(missing)?,
        fresh.pane.ok_or_else(missing)?,
    );
    let scope = layout.workspaces.get(at.0).ok_or_else(missing)?.id;
    let tab = layout.tab_mut(at)?;
    if tab.panes.len() >= MAX_PANES {
        return Err(LayoutError::LimitExceeded);
    }
    if !tab.replace(Child::Pane(pane), Child::Split(split)) {
        return Err(missing());
    }
    tab.splits.push(Split {
        id: split,
        axis,
        ratio,
        first: Child::Pane(pane),
        second: Child::Pane(created),
    });
    tab.panes.push(Pane {
        id: created,
        terminal: None,
    });
    Ok(Effect {
        created: vec![ResourceId::Split(split), ResourceId::Pane(created)],
        changed: vec![ResourceId::Pane(pane), ResourceId::Tab(tab.id)],
        scopes: vec![scope],
        tabs: vec![tab.id],
        ..Effect::default()
    })
}

pub(super) fn move_tab(
    layout: &mut Layout,
    tab: TabId,
    (workspace, index): (WorkspaceId, u32),
) -> Result<Effect, LayoutError> {
    let (source, position) = layout.tab_at(tab)?;
    let destination = layout.workspace_at(workspace)?;
    let index = usize::try_from(index).map_err(|_| LayoutError::InvalidParams {
        field: "edit.index",
    })?;
    if source == destination && position == index {
        return Ok(Effect::unchanged());
    }
    let capacity = layout
        .workspaces
        .get(destination)
        .ok_or_else(missing)?
        .tabs
        .len();
    let limit = if source == destination {
        capacity.saturating_sub(1)
    } else {
        capacity
    };
    if index > limit {
        return Err(LayoutError::InvalidParams {
            field: "edit.index",
        });
    }
    if source != destination && capacity >= MAX_TABS {
        return Err(LayoutError::LimitExceeded);
    }
    let from = layout.workspaces.get_mut(source).ok_or_else(missing)?;
    let source_id = from.id;
    let moved = from.tabs.remove(position);
    let to = layout.workspaces.get_mut(destination).ok_or_else(missing)?;
    to.tabs.insert(index, moved);
    Ok(Effect {
        changed: vec![
            ResourceId::Tab(tab),
            ResourceId::Workspace(source_id),
            ResourceId::Workspace(workspace),
        ],
        scopes: vec![source_id, workspace],
        tabs: vec![tab],
        ..Effect::default()
    })
}

pub(super) fn move_pane(
    layout: &mut Layout,
    (pane, workspace): (PaneId, WorkspaceId),
    fresh: Fresh,
) -> Result<Effect, LayoutError> {
    let at = layout.pane_at(pane)?;
    let destination = layout.workspace_at(workspace)?;
    let tab_id = fresh.tab.ok_or_else(missing)?;
    if layout
        .workspaces
        .get(destination)
        .ok_or_else(missing)?
        .tabs
        .len()
        >= MAX_TABS
    {
        return Err(LayoutError::LimitExceeded);
    }
    let source_id = layout.workspaces.get(at.0).ok_or_else(missing)?.id;
    let source = layout.tab_mut(at)?;
    let record = source
        .panes
        .iter()
        .find(|record| record.id == pane)
        .cloned()
        .ok_or_else(missing)?;
    let collapsed = source.detach(pane)?;
    let source_tab = source.id;
    layout
        .workspaces
        .get_mut(destination)
        .ok_or_else(missing)?
        .tabs
        .push(new_tab(tab_id, record));
    Ok(Effect {
        created: vec![ResourceId::Tab(tab_id)],
        removed: collapsed.into_iter().map(ResourceId::Split).collect(),
        changed: vec![
            ResourceId::Pane(pane),
            ResourceId::Tab(source_tab),
            ResourceId::Workspace(source_id),
            ResourceId::Workspace(workspace),
        ],
        scopes: vec![source_id, workspace],
        tabs: vec![source_tab, tab_id],
        ..Effect::default()
    })
}
