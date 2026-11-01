//! `session.snapshot`: terminals, agent bindings, leases and the layout in one answer.
//!
//! It is an unrecorded read answered off the executor: the agent bindings come from the
//! dispatch log's committed snapshot, and the terminals, their leases and the layout are
//! read in one job on the runtime's actor, which serializes every change to them. The
//! session is answered in one page; `page.limit` truncates the terminal list and says so.

use std::sync::Arc;

use ovis_terminal_agent::status::StatusService;
use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::layout::Layouts;
use ovis_terminal_core::leases::TerminalLeases;
use ovis_terminal_core::registry::{MachineId, TerminalId};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::methods::server::{
    SessionSnapshotParams, SessionSnapshotResult,
};
use ovis_terminal_server::dispatch::{Access, DispatchError, Read};
use ovis_terminal_server::streams::lease_record;
use ovis_terminal_store::Snapshot;
use serde_json::{Value, json};

use super::actor::{Actor, Runtime};
use super::error::{internal, refusal, worker_gone};

/// What the actor reads in one job.
pub(super) struct View {
    terminals: Vec<(TerminalId, Value)>,
    leases: Vec<Value>,
    layout: Value,
    revision: String,
    seq: String,
}

impl Runtime {
    /// The terminals, their leases and the layout, read in one actor job.
    pub(super) fn session_view(&self) -> Result<View, BoundaryError> {
        let layouts = Layouts::new(&self.layout_log, &self.authority, self.identity);
        let (records, layout) = self
            .control
            .with_terminals(|terminals, session| {
                Ok((terminals.list(session), layouts.get(session)))
            })
            .map_err(|error| refusal(error.kind(), error.hint()))?;
        let records = records.map_err(|error| super::terminal::terminal_refusal(&error))?;
        let layout = layout.map_err(|error| refusal(error.kind(), error.hint()))?;
        let terminals = records
            .iter()
            .map(|record| Ok((record.id(), self.record_value(record)?)))
            .collect::<Result<Vec<_>, BoundaryError>>()?;
        let mut leases = Vec::new();
        for live in self.live.values() {
            let view = live
                .hub
                .with_leases(TerminalLeases::view)
                .map_err(internal("read leases"))?;
            let held = view
                .interact
                .iter()
                .chain(view.geometry.iter())
                .chain(view.observers.iter());
            leases.extend(held.map(|lease| json!(lease_record(lease))));
        }
        let seq = self.journal.cursor().map_err(|error| error.boundary())?;
        Ok(View {
            terminals,
            leases,
            revision: layout.revision.get().to_string(),
            layout: super::layout::tree_of(&layout),
            seq: seq.get().to_string(),
        })
    }
}

/// `session.snapshot` as a dispatched read.
#[derive(Debug)]
pub(super) struct SessionRead {
    pub(super) actor: Actor,
    pub(super) status: Arc<StatusService>,
    pub(super) machine: MachineId,
}

impl Read for SessionRead {
    const NAME: &'static str = "session.snapshot";
    type Params = SessionSnapshotParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(Access::new(
            Action::Read,
            ResolvedTarget::new(self.machine, TargetResource::Machine),
        ))
    }

    fn read(&self, params: Self::Params, snapshot: &Snapshot) -> Result<Value, BoundaryError> {
        let view = self
            .actor
            .call_blocking(|runtime| runtime.session_view())
            .ok_or_else(worker_gone)??;
        let total = view.terminals.len();
        let limit = params
            .page
            .and_then(|page| page.limit)
            .map_or(total, |limit| usize::try_from(limit).unwrap_or(usize::MAX));
        let mut terminals = Vec::new();
        let mut agents = Vec::new();
        for (id, record) in view.terminals.into_iter().take(limit) {
            let bound = self
                .status
                .binding(snapshot, id)
                .map_err(|error| refusal(error.kind(), error.hint()))?;
            agents.extend(bound);
            terminals.push(record);
        }
        let value = json!({
            "revision": view.revision, "snapshot_seq": view.seq, "terminals": terminals,
            "agents": agents, "leases": view.leases, "layout": view.layout,
            "page": {"revision": view.revision, "total": u32::try_from(total).unwrap_or(u32::MAX),
                "next": null, "truncated": limit < total},
        });
        let checked: SessionSnapshotResult =
            serde_json::from_value(value).map_err(internal("encode session.snapshot"))?;
        serde_json::to_value(checked).map_err(internal("encode session.snapshot"))
    }
}
