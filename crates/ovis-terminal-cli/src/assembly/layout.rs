//! `layout.get`, `layout.edit` and `selection.set` over core's layout service, on the
//! runtime's layout log. A creating edit launches through the runtime's terminal
//! lifecycle, so the spawned program is served like one `terminal.run` started. Every
//! committed change is then published to `events.subscribe` subscribers (see `feed`).

pub(super) mod feed;
mod wire;

use ovis_terminal_authz::Session;
use ovis_terminal_core::events::{Coordinator, ResourceId};
use ovis_terminal_core::layout::{
    Call, EditRequest, LaunchOutcome, LayoutError, LayoutEvent, Layouts, Producer, SelectRequest,
};
use ovis_terminal_core::registry::WorkspaceId;
use ovis_terminal_protocol::errors::{BoundaryError, OpId};
use ovis_terminal_protocol::schema::methods::layout::{
    LayoutEditParams, LayoutTree, SelectionSetParams,
};
use ovis_terminal_protocol::schema::methods::terminal::{LaunchProgram, TerminalRunParams};
use ovis_terminal_protocol::schema::scalar::EventSeq;
use serde_json::{Value, json};

use super::actor::Runtime;
use super::error::refusal;
use super::terminal::{bounded, launch};

/// The runtime's layout log: layout rows and their operation ledger.
pub(super) type LayoutLog = Coordinator<LayoutEvent, Producer>;

/// Grid a pane's program starts at, as `terminal.run` does without one.
const PANE_GRID: (u16, u16) = (80, 24);

fn layout_refusal(error: &LayoutError) -> BoundaryError {
    refusal(error.kind(), error.hint())
}

fn encoded(value: &impl serde::Serialize) -> Result<Value, BoundaryError> {
    serde_json::to_value(value).map_err(|cause| super::error::internal("layout records")(cause))
}

/// The `terminal.run` parameters a pane's program launches with.
fn run_params(program: &LaunchProgram) -> Result<TerminalRunParams, BoundaryError> {
    wire::checked(json!({
        "op_id": uuid::Uuid::now_v7().to_string(), "program": program,
        "geometry": {"cols": PANE_GRID.0, "rows": PANE_GRID.1}, "cwd": null, "env": null,
        "name": null, "labels": null, "parent": null, "child_policy": null, "keep": null,
        "first_message": null, "machine": null,
    }))
}

impl Runtime {
    fn layouts(&self) -> Layouts<'_, LayoutEvent, Producer> {
        Layouts::new(&self.layout_log, &self.authority, self.identity)
    }

    /// The layout log position the last commit reached.
    fn layout_seq(&self) -> Result<EventSeq, BoundaryError> {
        let pinned = self
            .layout_log
            .pin_snapshot()
            .map_err(|cause| super::error::internal("read the layout log")(cause))?;
        let seq = EventSeq::new(pinned.cursor.seq.get());
        self.layout_log
            .release_snapshot(&pinned.token)
            .map_err(|cause| super::error::internal("read the layout log")(cause))?;
        Ok(seq)
    }

    fn answer(
        &self,
        session: &Session,
        change: &ovis_terminal_core::layout::Change,
    ) -> Result<Value, BoundaryError> {
        let after = self
            .layouts()
            .get(session)
            .map_err(|e| layout_refusal(&e))?;
        encoded(&wire::change(change, (&after, self.layout_seq()?))?)
    }

    /// `layout.get`: the layout and every client's selection at one revision.
    pub(super) fn layout_get(&self, session: &Session) -> Result<Value, BoundaryError> {
        let layout = self
            .layouts()
            .get(session)
            .map_err(|e| layout_refusal(&e))?;
        let tree: LayoutTree = wire::checked(wire::tree(&layout))?;
        encoded(&tree)
    }

    /// `layout.edit`: one explicit-ID edit; a creating edit with a program launches it.
    pub(super) fn layout_edit(
        &mut self,
        session: &Session,
        params: &LayoutEditParams,
    ) -> Result<Value, BoundaryError> {
        let (edit, program) = wire::edit(&params.edit)?;
        let mut request = EditRequest::new(edit);
        request.expect_revision = params.expect_revision;
        let op = OpId::new(params.op_id.as_str().to_owned());
        let Some(program) = program else {
            let change = self
                .layouts()
                .edit(Call::new(session, op), &request)
                .map_err(|e| layout_refusal(&e))?;
            self.publish_layout();
            return self.answer(session, &change);
        };
        let run = run_params(&program)?;
        bounded(&run)?;
        let grid = run.geometry;
        let (launched, _token) =
            launch(run).map_err(|error| super::terminal::terminal_refusal(&error))?;
        request.launch = Some(launched);
        let layouts = self.layouts();
        let change = self
            .control
            .with_terminals(|terminals, _| {
                Ok(layouts.edit(Call::new(session, op).launching(terminals), &request))
            })
            .map_err(|error| refusal(error.kind(), error.hint()))?
            .map_err(|e| layout_refusal(&e))?;
        if let LaunchOutcome::Launched { terminal, .. } = &change.launch {
            self.launches.insert(*terminal, (grid, None));
        }
        self.publish_layout();
        self.answer(session, &change)
    }

    /// `selection.set`: changes exactly the named client's selection.
    pub(super) fn select(
        &self,
        session: &Session,
        params: &SelectionSetParams,
    ) -> Result<Value, BoundaryError> {
        let request: SelectRequest = wire::checked(json!({
            "client": params.client_id, "workspace": params.workspace_id,
            "tab": params.tab_id, "pane": params.pane_id, "expect_revision": null,
        }))
        .map_err(|_| {
            refusal(
                "invalid_params",
                "name existing layout ids of the right kind",
            )
        })?;
        let op = OpId::new(params.op_id.as_str().to_owned());
        let change = self
            .layouts()
            .select(Call::new(session, op), &request)
            .map_err(|e| layout_refusal(&e))?;
        self.publish_layout();
        self.answer(session, &change)
    }

    /// Opens one workspace named `name` for a checkout, under the control's own session.
    pub(super) fn open_workspace(&self, name: &str) -> Result<WorkspaceId, BoundaryError> {
        let edit = wire::checked(json!({"op": "create_workspace", "name": name,
            "group_with": null}))?;
        let change = self.control_edit(&EditRequest::new(edit))?;
        change
            .created
            .iter()
            .find_map(|id| match id {
                ResourceId::Workspace(workspace) => Some(*workspace),
                _ => None,
            })
            .ok_or_else(|| refusal("internal", "the workspace edit created no workspace"))
    }

    /// Closes exactly `workspaces`, under the control's own session.
    pub(super) fn close_workspaces(&self, workspaces: &[WorkspaceId]) -> Result<(), BoundaryError> {
        for workspace in workspaces {
            let edit = wire::checked(json!({"op": "close",
                "target": {"kind": "workspace", "id": workspace}, "scope": "target"}))?;
            self.control_edit(&EditRequest::new(edit))?;
        }
        Ok(())
    }

    fn control_edit(
        &self,
        request: &EditRequest,
    ) -> Result<ovis_terminal_core::layout::Change, BoundaryError> {
        let layouts = self.layouts();
        let op = OpId::new(uuid::Uuid::now_v7().to_string());
        let change = self
            .control
            .with_terminals(|_, session| Ok(layouts.edit(Call::new(session, op), request)))
            .map_err(|error| refusal(error.kind(), error.hint()))?
            .map_err(|e| layout_refusal(&e))?;
        self.publish_layout();
        Ok(change)
    }
}

/// The published tree of `layout`.
pub(super) fn tree_of(layout: &ovis_terminal_core::layout::Layout) -> Value {
    wire::tree(layout)
}
