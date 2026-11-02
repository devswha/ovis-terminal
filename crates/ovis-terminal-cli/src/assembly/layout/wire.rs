//! Translation between the published layout records and core's layout model.
//!
//! Core splits are binary and its panes are leaves, so a published tab lists one node per
//! split (two children) and one per pane (keyed by the pane's own id). Core has no tab
//! names and no rename edit: tabs publish no name and `rename` is refused.

use ovis_terminal_core::events::ResourceId;
use ovis_terminal_core::layout::{Change, Child, LaunchOutcome, Layout, Tab};
use ovis_terminal_core::terminals::LaunchState;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::methods::layout::{LayoutChange, LayoutEdit};
use ovis_terminal_protocol::schema::methods::terminal::LaunchProgram;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::assembly::error::refusal;

fn internal<E: std::fmt::Debug>(cause: E) -> BoundaryError {
    crate::assembly::error::internal("layout records")(cause)
}

/// Decodes `value` as the published or core type `T`, so every record is checked.
pub(super) fn checked<T: DeserializeOwned>(value: Value) -> Result<T, BoundaryError> {
    serde_json::from_value(value).map_err(internal)
}

/// A child's node id: the id of the split or pane it is, as core serializes it.
fn node(child: &Child) -> Value {
    serde_json::to_value(child)
        .ok()
        .and_then(|tagged| tagged.get("id").cloned())
        .unwrap_or(Value::Null)
}

const fn axis(axis: ovis_terminal_core::layout::Axis) -> &'static str {
    match axis {
        ovis_terminal_core::layout::Axis::Vertical => "vertical",
        _ => "horizontal",
    }
}

fn tab(tab: &Tab) -> Value {
    let splits = tab.splits.iter().map(|split| {
        json!({"node_id": split.id.to_string(), "axis": axis(split.axis),
            "ratio_permille": split.ratio.permille(),
            "children": [node(&split.first), node(&split.second)], "pane_id": null})
    });
    let leaves = tab.panes.iter().map(|pane| {
        json!({"node_id": pane.id.to_string(), "axis": null, "ratio_permille": null,
            "children": [], "pane_id": pane.id.to_string()})
    });
    let panes: Vec<Value> = tab
        .panes
        .iter()
        .map(|pane| {
            json!({"pane_id": pane.id.to_string(),
                "terminal_id": pane.terminal.map(|terminal| terminal.to_string()),
                "zoomed": tab.zoomed == Some(pane.id)})
        })
        .collect();
    let root = tab
        .root
        .as_ref()
        .map_or_else(|| json!(tab.id.to_string()), node);
    json!({"tab_id": tab.id.to_string(), "name": null, "root": root,
        "nodes": splits.chain(leaves).collect::<Vec<_>>(), "panes": panes})
}

/// The published tree of `layout`.
pub(super) fn tree(layout: &Layout) -> Value {
    let workspaces: Vec<Value> = layout
        .workspaces
        .iter()
        .map(|workspace| {
            json!({"workspace_id": workspace.id.to_string(), "name": workspace.name,
                "tabs": workspace.tabs.iter().map(tab).collect::<Vec<_>>()})
        })
        .collect();
    let selections: Vec<Value> = layout
        .selections
        .iter()
        .map(|selection| {
            json!({"client_id": selection.client,
                "workspace_id": selection.workspace.map(|id| id.to_string()),
                "tab_id": selection.tab.map(|id| id.to_string()),
                "pane_id": selection.pane.map(|id| id.to_string())})
        })
        .collect();
    json!({"revision": layout.revision, "workspaces": workspaces, "selections": selections})
}

pub(super) fn resource(id: &ResourceId) -> Option<Value> {
    match id {
        ResourceId::Workspace(id) => Some(json!({"workspace_id": id.to_string()})),
        ResourceId::Tab(id) => Some(json!({"tab_id": id.to_string()})),
        ResourceId::Pane(id) => Some(json!({"pane_id": id.to_string()})),
        ResourceId::Split(id) => Some(json!({"node_id": id.to_string()})),
        _ => None,
    }
}

/// The published change: the layout after it, what it created and removed, the launch's
/// spawn progress and the layout log position that committed it.
pub(super) fn change(
    change: &Change,
    (after, seq): (&Layout, EventSeq),
) -> Result<LayoutChange, BoundaryError> {
    let spawn = match &change.launch {
        LaunchOutcome::Launched { spawn, .. } => Some(match spawn {
            LaunchState::Spawned => "spawned",
            LaunchState::Failed => "failed",
            _ => "pending",
        }),
        LaunchOutcome::Rejected { .. } => Some("failed"),
        _ => None,
    };
    checked(json!({
        "layout": tree(after),
        "no_op": change.no_op.map(|_| "already_applied"),
        "created": change.created.iter().filter_map(resource).collect::<Vec<_>>(),
        "removed": change.removed.iter().filter_map(resource).collect::<Vec<_>>(),
        "spawn": spawn,
        "seq": seq,
    }))
}

/// The core target of a published resource; a split node cannot be closed.
fn target(resource: &Value) -> Result<Value, BoundaryError> {
    let named = |kind: &str, field: &str| {
        resource
            .get(field)
            .map(|id| json!({"kind": kind, "id": id}))
    };
    named("workspace", "workspace_id")
        .or_else(|| named("tab", "tab_id"))
        .or_else(|| named("pane", "pane_id"))
        .ok_or_else(|| refusal("invalid_params", "close a workspace, tab or pane"))
}

/// The core edit and the program a creating edit launches.
pub(super) fn edit(
    edit: &LayoutEdit,
) -> Result<(ovis_terminal_core::layout::Edit, Option<LaunchProgram>), BoundaryError> {
    let published = serde_json::to_value(edit).map_err(internal)?;
    let field = |name: &str| published.get(name).cloned().unwrap_or(Value::Null);
    let program = checked::<Option<LaunchProgram>>(field("program"))?;
    let core = match published.get("op").and_then(Value::as_str) {
        Some("create_workspace") => {
            json!({"op": "create_workspace", "name": field("name"), "group_with": null})
        }
        Some("create_tab") => json!({"op": "create_tab", "workspace": field("workspace_id")}),
        Some("split_pane") => json!({"op": "split", "pane": field("pane_id"),
            "axis": field("axis"), "ratio_permille": 500}),
        Some("close") => json!({"op": "close", "target": target(&field("resource"))?,
            "scope": field("scope")}),
        Some("set_ratio") => json!({"op": "set_ratio", "split": field("node_id"),
            "ratio_permille": field("ratio_permille")}),
        Some("swap_panes") => json!({"op": "swap", "first": field("first"),
            "second": field("second")}),
        Some("zoom_pane") => json!({"op": "zoom", "pane": field("pane_id"),
            "zoomed": field("zoomed")}),
        Some("move_tab") => json!({"op": "move_tab", "tab": field("tab_id"),
            "workspace": field("workspace_id"), "index": field("index")}),
        _ => {
            return Err(refusal(
                "unsupported_field",
                "this runtime's layouts have no names to change",
            ));
        }
    };
    let core = serde_json::from_value(core).map_err(|_| {
        refusal(
            "invalid_params",
            "name existing layout ids of the right kind",
        )
    })?;
    Ok((core, program))
}
