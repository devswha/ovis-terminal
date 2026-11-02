//! The wire data of `layout.changed` and `selection.changed`, typed as the published
//! `LayoutChangedData` and `SelectionChangedData` before it is committed.

use ovis_terminal_core::layout::{LayoutEvent, SelectionChanged};
use ovis_terminal_protocol::schema::methods::layout::{LayoutChangedData, SelectionChangedData};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::FeedError;
use crate::assembly::layout::wire;

/// A layout record as subscribers receive it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub(in crate::assembly) enum Published {
    /// `layout.changed`.
    Changed(LayoutChangedData),
    /// `selection.changed`.
    Selection(SelectionChangedData),
}

/// The terminal `pane` shows in `tree`, if any.
fn terminal_of(tree: &Value, pane: &str) -> Value {
    let panes = tree
        .get("workspaces")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|workspace| workspace.get("tabs").and_then(Value::as_array))
        .flatten()
        .filter_map(|tab| tab.get("panes").and_then(Value::as_array))
        .flatten();
    panes
        .into_iter()
        .find(|record| record.get("pane_id").and_then(Value::as_str) == Some(pane))
        .and_then(|record| record.get("terminal_id").cloned())
        .unwrap_or(Value::Null)
}

fn selection(
    changed: &SelectionChanged,
    (op_id, tree): (&str, &Value),
) -> Result<SelectionChangedData, serde_json::Error> {
    let pane = changed.pane.map(|id| id.to_string());
    let terminal = pane
        .as_deref()
        .map_or(Value::Null, |pane| terminal_of(tree, pane));
    serde_json::from_value(json!({
        "client_id": changed.client,
        "workspace_id": changed.workspace.map(|id| id.to_string()),
        "tab_id": changed.tab.map(|id| id.to_string()),
        "pane_id": pane,
        "terminal_id": terminal,
        "op_id": op_id,
        "revision": tree.get("revision"),
        "cause": changed.cause,
    }))
}

impl Published {
    /// What subscribers receive for `event` of operation `op_id`, with `tree` the layout
    /// at the log's head; `None` for records no subscriber is offered.
    pub(super) fn of(
        event: &LayoutEvent,
        (op_id, tree): (&str, &Value),
    ) -> Result<Option<Self>, FeedError> {
        let field = |name: &str| tree.get(name).cloned().unwrap_or(Value::Null);
        let resources = |ids: &[ovis_terminal_core::events::ResourceId]| {
            ids.iter().filter_map(wire::resource).collect::<Vec<_>>()
        };
        let published = match event {
            LayoutEvent::Changed(changed) => Self::Changed(
                serde_json::from_value(json!({
                    "revision": field("revision"), "workspaces": field("workspaces"),
                    "selections": field("selections"), "op_id": op_id,
                    "created": resources(&changed.created),
                    "removed": resources(&changed.removed),
                    "changed": resources(&changed.changed),
                }))
                .map_err(FeedError::Record)?,
            ),
            LayoutEvent::Selection(changed) => {
                Self::Selection(selection(changed, (op_id, tree)).map_err(FeedError::Record)?)
            }
            _ => return Ok(None),
        };
        Ok(Some(published))
    }

    /// Its event type.
    pub(in crate::assembly) const fn event_type(&self) -> &'static str {
        match self {
            Self::Changed(_) => "layout.changed",
            Self::Selection(_) => "selection.changed",
        }
    }

    /// Its data, as an `event` notification carries it.
    pub(in crate::assembly) fn payload(&self) -> Result<Value, serde_json::Error> {
        match self {
            Self::Changed(data) => serde_json::to_value(data),
            Self::Selection(data) => serde_json::to_value(data),
        }
    }
}
