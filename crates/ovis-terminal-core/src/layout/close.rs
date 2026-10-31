//! Close with explicit scope; linked workspaces close only under group scope.

use super::LayoutError;
use super::edit::{CloseScope, Effect, Target, missing};
use super::model::Layout;
use crate::events::ResourceId;
use crate::registry::WorkspaceId;

pub(super) fn close(
    layout: &mut Layout,
    target: Target,
    scope: CloseScope,
) -> Result<Effect, LayoutError> {
    match target {
        Target::Pane(pane) => {
            let at = layout.pane_at(pane)?;
            let workspace = layout.workspaces.get(at.0).ok_or_else(missing)?.id;
            let tab = layout.tab_mut(at)?;
            let released = tab.panes.iter().filter(|record| record.id == pane);
            let released = released.filter_map(|record| record.terminal).collect();
            let collapsed = tab.detach(pane)?;
            let mut removed = vec![ResourceId::Pane(pane)];
            removed.extend(collapsed.map(ResourceId::Split));
            Ok(Effect {
                removed,
                changed: vec![ResourceId::Tab(tab.id)],
                released,
                scopes: vec![workspace],
                tabs: vec![tab.id],
                ..Effect::default()
            })
        }
        Target::Tab(tab) => {
            let (index, position) = layout.tab_at(tab)?;
            let owner = layout.workspaces.get_mut(index).ok_or_else(missing)?;
            let closed = owner.tabs.remove(position);
            Ok(Effect {
                removed: closed.contents(),
                changed: vec![ResourceId::Workspace(owner.id)],
                released: closed.terminals().collect(),
                scopes: vec![owner.id],
                ..Effect::default()
            })
        }
        Target::Workspace(workspace) => close_workspace(layout, workspace, scope),
    }
}

fn close_workspace(
    layout: &mut Layout,
    workspace: WorkspaceId,
    scope: CloseScope,
) -> Result<Effect, LayoutError> {
    let index = layout.workspace_at(workspace)?;
    let group = layout.workspaces.get(index).ok_or_else(missing)?.group;
    let members: Vec<WorkspaceId> = layout
        .workspaces
        .iter()
        .filter(|candidate| {
            candidate.id == workspace || (group.is_some() && candidate.group == group)
        })
        .map(|candidate| candidate.id)
        .collect();
    let mut effect = Effect::default();
    for closed in layout.workspaces.iter().filter(|w| members.contains(&w.id)) {
        effect.removed.push(ResourceId::Workspace(closed.id));
        for tab in &closed.tabs {
            effect.removed.extend(tab.contents());
            effect.released.extend(tab.terminals());
        }
    }
    if members.len() > 1 && scope == CloseScope::Target {
        return Err(LayoutError::GroupScopeRequired {
            affected: effect.affected(),
        });
    }
    layout
        .workspaces
        .retain(|candidate| !members.contains(&candidate.id));
    effect.scopes = members;
    Ok(effect)
}
