//! `layout.get`, `layout.edit` and `selection.set` on the real binary: a workspace created
//! with a program binds its pane to a terminal the runtime serves, a split adds a pane and
//! a node, a closed pane is removed with the split that held it, one client's selection
//! changes alone, and a rename is refused (core layouts carry no names to change).

use serde_json::{Value, json};

use super::support::{Served, TestResult, op_id};

fn call(served: &Served, method: &str, params: &Value) -> TestResult<Value> {
    served.ok(&["call", method, "--params", &params.to_string()])
}

fn edit(served: &Served, edit: &Value) -> TestResult<Value> {
    call(
        served,
        "layout.edit",
        &json!({"op_id": op_id()?, "expect_revision": null, "edit": edit}),
    )
}

fn panes(layout: &Value) -> Vec<Value> {
    layout
        .pointer("/workspaces/0/tabs/0/panes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Creates workspace `main` without a program and returns its layout.
fn workspace(served: &Served) -> TestResult<Value> {
    let created = edit(
        served,
        &json!({"op": "create_workspace", "name": "main", "program": null}),
    )?;
    Ok(created
        .get("layout")
        .cloned()
        .ok_or_else(|| created.to_string())?)
}

#[test]
fn a_workspace_launches_its_program_a_split_adds_a_pane_and_a_close_removes_it() -> TestResult {
    let served = Served::start()?;
    let empty = call(&served, "layout.get", &json!({}))?;
    assert_eq!(empty.get("workspaces"), Some(&json!([])), "{empty}");
    let created = edit(
        &served,
        &json!({"op": "create_workspace", "name": "main", "program": {"argv": ["/bin/sh"]}}),
    )?;
    let layout = created
        .get("layout")
        .cloned()
        .ok_or_else(|| created.to_string())?;
    assert_eq!(
        layout.pointer("/workspaces/0/name"),
        Some(&json!("main")),
        "{created}"
    );
    let first = panes(&layout);
    let terminal = first
        .first()
        .and_then(|pane| pane.get("terminal_id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| created.to_string())?;
    assert!(
        created.get("spawn").is_some_and(Value::is_string),
        "{created}"
    );
    let listed = call(&served, "terminal.list", &json!({}))?;
    assert!(listed.to_string().contains(&terminal), "{listed}");

    let pane = first
        .first()
        .and_then(|pane| pane.get("pane_id"))
        .cloned()
        .ok_or("pane")?;
    let split = edit(
        &served,
        &json!({"op": "split_pane", "pane_id": pane, "axis": "vertical", "program": null}),
    )?;
    let after = split
        .get("layout")
        .cloned()
        .ok_or_else(|| split.to_string())?;
    assert_eq!(panes(&after).len(), 2, "{split}");
    let nodes = after
        .pointer("/workspaces/0/tabs/0/nodes")
        .and_then(Value::as_array);
    assert_eq!(nodes.map(Vec::len), Some(3), "{split}");

    let second = panes(&after)
        .into_iter()
        .filter_map(|pane| pane.get("pane_id").cloned())
        .find(|id| *id != pane)
        .ok_or("second pane")?;
    let closed = edit(
        &served,
        &json!({"op": "close", "resource": {"pane_id": second}, "scope": "target"}),
    )?;
    let split_node = after
        .pointer("/workspaces/0/tabs/0/nodes")
        .and_then(Value::as_array)
        .and_then(|nodes| {
            nodes
                .iter()
                .find(|node| node.get("pane_id") == Some(&Value::Null))
        })
        .and_then(|node| node.get("node_id").cloned())
        .ok_or("split node")?;
    assert_eq!(
        closed.get("removed"),
        Some(&json!([{"pane_id": second}, {"node_id": split_node}])),
        "the pane and the split that held it: {closed}"
    );
    let remaining = closed.get("layout").map(panes).unwrap_or_default();
    assert_eq!(remaining.len(), 1, "{closed}");
    served.stop()?;
    Ok(())
}

#[test]
fn a_selection_changes_only_the_named_client_and_a_rename_is_refused() -> TestResult {
    let served = Served::start()?;
    let layout = workspace(&served)?;
    let id = layout
        .pointer("/workspaces/0/workspace_id")
        .cloned()
        .ok_or("workspace")?;
    let selected = call(
        &served,
        "selection.set",
        &json!({"op_id": op_id()?, "client_id": "phone", "workspace_id": id,
            "tab_id": null, "pane_id": null}),
    )?;
    let selections = selected
        .pointer("/layout/selections")
        .cloned()
        .unwrap_or_default();
    assert_eq!(selections.as_array().map(Vec::len), Some(1), "{selected}");
    assert_eq!(
        selections.pointer("/0/client_id"),
        Some(&json!("phone")),
        "{selected}"
    );
    let rename = served.refused(
        &[
            "call",
            "layout.edit",
            "--params",
            &json!({"op_id": op_id()?, "expect_revision": null,
                "edit": {"op": "rename", "resource": {"workspace_id": id}, "name": "x"}})
            .to_string(),
        ],
        1,
    )?;
    assert_eq!(
        rename.get("kind"),
        Some(&json!("unsupported_field")),
        "{rename}"
    );
    served.stop()?;
    Ok(())
}
