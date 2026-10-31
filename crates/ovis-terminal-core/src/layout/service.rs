//! The layout transaction boundary: operation ledger, authority and one commit unit.

use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::{
    AccessRequest, Action, Authority, ResolvedTarget, Session, TargetResource,
};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::scalar::{Revision, TimestampMs};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::plan::{plan_bind, plan_edit, plan_select};
use super::records::{Change, LayoutEvent};
use super::unit::{Begun, begin, fail, record, settle};
use super::{Edit, Launcher, Layout, LayoutError, SelectRequest};
use crate::events::{Coordinator, EventData, SnapshotProducer};
use crate::ops::{Context, OperationError, OperationEvent, OperationSnapshot, Operations, Request};
use crate::registry::{RegistryIdentities, RuntimeIdentity, WorkspaceId};
use crate::terminals::Launch;

/// Parameters of `layout.edit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EditRequest {
    /// Layout revision the edit was computed against.
    pub expect_revision: Option<Revision>,
    /// The edit.
    pub edit: Edit,
    /// Program for the pane a creating edit makes.
    pub launch: Option<Launch>,
}

impl EditRequest {
    /// An edit without a revision guard or launch.
    pub const fn new(edit: Edit) -> Self {
        Self {
            expect_revision: None,
            edit,
            launch: None,
        }
    }

    /// Parses the closed request schema once at the boundary.
    ///
    /// # Errors
    ///
    /// Returns `invalid_params` for unknown fields, numeric counters or malformed IDs.
    pub fn decode(bytes: &[u8]) -> Result<Self, LayoutError> {
        serde_json::from_slice(bytes).map_err(LayoutError::InvalidJson)
    }
}

/// An authenticated request, its operation identity and an optional launcher.
#[derive(Debug)]
pub struct Call<'a> {
    session: &'a Session,
    op_id: OpId,
    launcher: Option<&'a mut dyn Launcher>,
}

impl<'a> Call<'a> {
    /// Binds a request to its session and client-minted operation identity.
    pub const fn new(session: &'a Session, op_id: OpId) -> Self {
        Self {
            session,
            op_id,
            launcher: None,
        }
    }

    /// Supplies the launcher used when the edit asks for a program.
    #[must_use]
    pub fn launching(mut self, launcher: &'a mut dyn Launcher) -> Self {
        self.launcher = Some(launcher);
        self
    }
}

/// Explicit-ID layout service over a shared commit coordinator.
#[derive(Debug)]
pub struct Layouts<'a, D, P> {
    core: &'a Coordinator<D, P>,
    authority: &'a Authority<RegistryIdentities>,
    identity: RuntimeIdentity,
}

impl<'a, D, P> Layouts<'a, D, P> {
    /// Attaches layout operations to the core's coordinator and authority.
    pub const fn new(
        core: &'a Coordinator<D, P>,
        authority: &'a Authority<RegistryIdentities>,
        identity: RuntimeIdentity,
    ) -> Self {
        Self {
            core,
            authority,
            identity,
        }
    }
}

impl<D, P> Layouts<'_, D, P>
where
    D: EventData + From<OperationEvent> + From<LayoutEvent> + Clone + Serialize + DeserializeOwned,
    P: SnapshotProducer,
    P::Sections: AsRef<OperationSnapshot> + AsRef<Layout> + DeserializeOwned,
{
    /// Reads the layout and every client's selection at one revision.
    ///
    /// # Errors
    ///
    /// Returns authority or snapshot failures.
    pub fn get(&self, session: &Session) -> Result<Layout, LayoutError> {
        self.authorize(session, (Action::Read, &[]))?;
        let snapshot = self.core.read_snapshot()?;
        Ok(AsRef::<Layout>::as_ref(&snapshot.sections).clone())
    }

    /// Applies one explicit-ID edit; a creation commits before any launch is tried.
    ///
    /// # Errors
    ///
    /// Returns typed validation, geometry, scope, revision, authority or operation
    /// failures. A failed launch is a [`super::LaunchOutcome`], never an error.
    pub fn edit(&self, call: Call<'_>, request: &EditRequest) -> Result<Change, LayoutError> {
        let Call {
            session,
            op_id,
            launcher,
        } = call;
        if request.launch.is_some() && !request.edit.creates_pane() {
            return Err(LayoutError::InvalidParams { field: "launch" });
        }
        let ops = self.operations(session)?;
        let params = serde_json::to_value(request)?;
        let permit = match begin(&ops, &Request::new(op_id, "layout.edit".into(), params)?)? {
            Begun::Replayed(change) => return Ok(*change),
            Begun::Started(permit) => permit,
        };
        let runtime = self.identity.runtime_id();
        let plan = |snapshot: &_| {
            let (planned, scopes) = plan_edit(snapshot, runtime, request)?;
            self.authorize(session, (Action::LayoutChange, &scopes))?;
            Ok(planned)
        };
        let (Some(launch), Some(launcher)) = (request.launch.clone(), launcher) else {
            if request.launch.is_some() {
                return Err(fail(&ops, permit, LayoutError::LauncherUnavailable));
            }
            return settle(&ops, permit, plan);
        };
        let created = match record(&ops, &permit, plan) {
            Ok(change) => change,
            Err(error) => return Err(fail(&ops, permit, error)),
        };
        let outcome = launcher.launch(session, launch);
        settle(&ops, permit, |snapshot| {
            plan_bind(snapshot, runtime, (created, outcome))
        })
    }

    /// Changes exactly one client's selection; every other selection is untouched.
    ///
    /// # Errors
    ///
    /// Returns `not_found`, `invalid_params`, `revision_mismatch`, authority or operation failures.
    pub fn select(&self, call: Call<'_>, request: &SelectRequest) -> Result<Change, LayoutError> {
        let ops = self.operations(call.session)?;
        let params = serde_json::to_value(request)?;
        let started = begin(
            &ops,
            &Request::new(call.op_id, "selection.set".into(), params)?,
        )?;
        let permit = match started {
            Begun::Replayed(change) => return Ok(*change),
            Begun::Started(permit) => permit,
        };
        let ids = (self.identity.runtime_id(), self.identity.machine_id());
        settle(&ops, permit, |snapshot| {
            let scopes: Vec<WorkspaceId> = request.workspace.into_iter().collect();
            self.authorize(call.session, (Action::Read, &scopes))?;
            plan_select(snapshot, ids, request)
        })
    }

    fn operations(&self, session: &Session) -> Result<Operations<'_, D, P>, LayoutError> {
        let context = Context::new(session.principal().clone(), self.identity, now()?);
        Ok(Operations::new(self.core, context))
    }

    fn authorize(
        &self,
        session: &Session,
        (action, scopes): (Action, &[WorkspaceId]),
    ) -> Result<(), LayoutError> {
        let machine = self.identity.machine_id();
        let targets: Vec<TargetResource<RegistryIdentities>> = if scopes.is_empty() {
            vec![TargetResource::Machine]
        } else {
            let workspaces = scopes.iter().copied();
            workspaces
                .map(|workspace| TargetResource::Workspace { workspace })
                .collect()
        };
        for target in targets {
            let request = AccessRequest::new(action, ResolvedTarget::new(machine, target), now()?);
            self.authority.authorize(session, &request)?;
        }
        Ok(())
    }
}

fn now() -> Result<TimestampMs, OperationError> {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    Ok(TimestampMs::new(
        u64::try_from(millis).map_err(|_| OperationError::InvalidParams)?,
    ))
}
