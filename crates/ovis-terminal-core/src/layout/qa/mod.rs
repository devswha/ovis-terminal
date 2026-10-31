//! Real SQLite, authority and operation-ledger fixtures for layout integration.

mod adversarial;
mod behavior;
#[cfg(unix)]
mod launch;

use std::path::PathBuf;
use std::sync::Arc;

pub use adversarial::{
    malformed_requests_are_rejected_before_any_effect, replay_converges_and_reuse_is_refused,
    session_snapshot_holds_layout_and_terminals_at_one_revision,
};
pub use behavior::{failure, happy};
#[cfg(unix)]
pub use launch::{launch_binds_terminal_to_created_pane, layout_never_mutates_a_checkout};
use ovis_terminal_authz::{
    Authority, AuthorityConfig, EndpointPolicy, LocalAccount, OwnerSpec, PeerEvidence,
    Presentation, PrincipalId, RemoteTransport, RuntimeBinding, Session, UnixUid,
};
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::layout::{
    Call, Change, Edit, EditRequest, Layout, LayoutError, LayoutEvent, Layouts, Producer,
    SelectRequest,
};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, PaneId, RegistryIdentities, RuntimeId, RuntimeIdentity,
    ServerBootId, SplitId, TabId, WorkspaceId,
};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::ClientId;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_store::{Store, StoreOptions};
use uuid::Uuid;

/// Outcome of an assertion-bearing layout case.
pub type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Fixture {
    root: PathBuf,
    core: Option<Coordinator<LayoutEvent, Producer>>,
    authority: Arc<Authority<RegistryIdentities>>,
    session: Session,
    identity: RuntimeIdentity,
}

#[derive(Debug, Clone, Copy)]
struct Created {
    workspace: Option<WorkspaceId>,
    tab: Option<TabId>,
    pane: Option<PaneId>,
    split: Option<SplitId>,
}

fn created(change: &Change) -> Created {
    use ovis_terminal_core::events::ResourceId;
    let mut ids = Created {
        workspace: None,
        tab: None,
        pane: None,
        split: None,
    };
    for id in &change.created {
        match id {
            ResourceId::Workspace(id) => ids.workspace = Some(*id),
            ResourceId::Tab(id) => ids.tab = Some(*id),
            ResourceId::Pane(id) => ids.pane = Some(*id),
            ResourceId::Split(id) => ids.split = Some(*id),
            _ => {}
        }
    }
    ids
}

fn op() -> OpId {
    OpId::new(Uuid::now_v7().to_string())
}

impl Fixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!("ovis-t18-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&root)?;
        let identity = RuntimeIdentity::new(
            HostIdentity::new(
                MachineId::new(Uuid::from_u128(18))?,
                HostBootId::try_from(1)?,
            ),
            RuntimeId::try_from(1)?,
            ServerBootId::try_from(1)?,
        );
        let (authority, owner) = Authority::<RegistryIdentities>::bootstrap(AuthorityConfig::new(
            RuntimeBinding::new(identity.machine_id(), identity.runtime_id()),
            OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(u64::MAX)),
        ))?;
        let admission = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1)))
            .admit(&PeerEvidence::Remote(RemoteTransport::Ssh))?;
        let session = authority.authenticate(
            &admission,
            Presentation::new(owner.bearer(), TimestampMs::new(1)),
        )?;
        let store = Store::open(&root.join("layout"), StoreOptions::default())?;
        Ok(Self {
            core: Some(Coordinator::new(
                store,
                identity,
                Producer::new(identity.runtime_id()),
            )),
            root,
            authority: Arc::new(authority),
            session,
            identity,
        })
    }

    fn core(&self) -> Result<&Coordinator<LayoutEvent, Producer>, Box<dyn std::error::Error>> {
        self.core
            .as_ref()
            .ok_or_else(|| "coordinator is closed".into())
    }

    fn layouts(&self) -> Result<Layouts<'_, LayoutEvent, Producer>, Box<dyn std::error::Error>> {
        Ok(Layouts::new(self.core()?, &self.authority, self.identity))
    }

    fn attempt(
        &self,
        request: &EditRequest,
    ) -> Result<Result<Change, LayoutError>, Box<dyn std::error::Error>> {
        Ok(self
            .layouts()?
            .edit(Call::new(&self.session, op()), request))
    }

    fn edit(&self, edit: Edit) -> Result<Change, Box<dyn std::error::Error>> {
        Ok(self.attempt(&EditRequest::new(edit))??)
    }

    fn select(
        &self,
        client: &str,
        ids: (Option<WorkspaceId>, Option<TabId>, Option<PaneId>),
    ) -> Result<Change, Box<dyn std::error::Error>> {
        let request = SelectRequest::new(ClientId::new(client.into()), ids);
        Ok(self
            .layouts()?
            .select(Call::new(&self.session, op()), &request)?)
    }

    fn layout(&self) -> Result<Layout, Box<dyn std::error::Error>> {
        Ok(self.layouts()?.get(&self.session)?)
    }

    fn reopen_store(&mut self) -> Result<Store, Box<dyn std::error::Error>> {
        self.core.take().ok_or("coordinator")?.close()?;
        Ok(Store::open(
            &self.root.join("layout"),
            StoreOptions::default(),
        )?)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(core) = self.core.take() {
            let closed = core.close();
            assert!(closed.is_ok(), "close coordinator: {closed:?}");
        }
        let removed = std::fs::remove_dir_all(&self.root);
        assert!(removed.is_ok(), "remove owned fixture: {removed:?}");
    }
}

fn workspace_with_split(
    fixture: &Fixture,
) -> Result<(Created, Created), Box<dyn std::error::Error>> {
    let first = created(&fixture.edit(Edit::CreateWorkspace {
        name: "main".into(),
        group_with: None,
    })?);
    let split = created(&fixture.edit(Edit::Split {
        pane: first.pane.ok_or("pane")?,
        axis: ovis_terminal_core::layout::Axis::Horizontal,
        ratio_permille: 500,
    })?);
    Ok((first, split))
}

fn tab_shape(layout: &Layout, tab: TabId) -> Option<(Vec<PaneId>, Vec<SplitId>)> {
    let found = layout
        .workspaces
        .iter()
        .flat_map(|w| &w.tabs)
        .find(|t| t.id == tab)?;
    let mut panes: Vec<PaneId> = found.panes.iter().map(|p| p.id).collect();
    let mut splits: Vec<SplitId> = found.splits.iter().map(|s| s.id).collect();
    panes.sort();
    splits.sort();
    Some((panes, splits))
}
