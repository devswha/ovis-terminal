//! The worktree lane over a real Git repository (plan todo 52).
//!
//! The fixture's main dispatcher sequences agent status events, which carry no worktree
//! variant, so `worktree.*` is served by its own dispatcher whose event domain is
//! operation and worktree events, over its own store, the runtime's authority and the
//! same served connection. No production workspace owner exists before assembly (todo 66),
//! so opened workspaces are recorded by a fixture owner. `fixture.worktrees` dirties a
//! checkout this lane created, as a user editing it would.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use ovis_terminal_authz::Authority;
use ovis_terminal_core::events::{Coordinator, EventData};
use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
use ovis_terminal_core::registry::{RegistryIdentities, RuntimeIdentity, WorkspaceId};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_protocol::schema::ids::CheckoutId;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_server::dispatch::Dispatcher;
use ovis_terminal_server::dispatch::lanes::worktrees::Lane;
use ovis_terminal_services::worktrees::{
    Config, Git, Repository, WorkspaceFailure, Workspaces, WorktreeEvent, Worktrees,
};
use ovis_terminal_store::{Store, StoreOptions};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::machines::internal;
use super::serve::at;
use crate::error::RunnerError;

/// The unadvertised control that edits a checkout.
pub(crate) const DIRTY: &str = "fixture.worktrees";

/// Operation and worktree events on the worktree lane's coordinator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum WorktreeDomain {
    Worktree(WorktreeEvent),
    Operation(Box<OperationEvent>),
}

impl From<OperationEvent> for WorktreeDomain {
    fn from(event: OperationEvent) -> Self {
        Self::Operation(Box::new(event))
    }
}

impl From<WorktreeEvent> for WorktreeDomain {
    fn from(event: WorktreeEvent) -> Self {
        Self::Worktree(event)
    }
}

impl EventData for WorktreeDomain {
    fn event_type(&self) -> &'static str {
        match self {
            Self::Worktree(event) => event.event_type(),
            Self::Operation(event) => event.event_type(),
        }
    }

    fn at_sequence(&mut self, seq: EventSeq) {
        if let Self::Operation(event) = self {
            event.at_sequence(seq);
        }
    }
}

type Core = Coordinator<WorktreeDomain, OperationProducer>;
pub(crate) type WorktreeDispatch = Dispatcher<WorktreeDomain, OperationProducer>;

/// Workspaces opened for checkouts, numbered in opening order.
#[derive(Debug, Default)]
struct Opened(Mutex<u64>);

impl Workspaces for Opened {
    fn open(&self, _: &CheckoutId, _: &Path) -> Result<WorkspaceId, WorkspaceFailure> {
        let mut opened = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *opened = opened
            .checked_add(1)
            .ok_or("workspace numbering overflowed")?;
        Ok(WorkspaceId::try_from(*opened)?)
    }

    fn close(&self, _: &[WorkspaceId]) -> Result<(), WorkspaceFailure> {
        Ok(())
    }
}

/// The worktree lane's dispatcher and checkout directory.
#[derive(Debug)]
pub(crate) struct WorktreeSide {
    pub(crate) dispatch: Arc<WorktreeDispatch>,
    checkouts: PathBuf,
}

fn git(repo: &Path, args: &[&str]) -> Result<(), RunnerError> {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .map_err(at("run git for the fixture repository"))?;
    if status.success() {
        Ok(())
    } else {
        Err(at("prepare the fixture repository")(format!(
            "git {args:?}: {status}"
        )))
    }
}

/// A repository with one commit below `<root>/wt` and the worktree lane over it.
pub(crate) fn serve(
    root: &Path,
    authority: &Arc<Authority<RegistryIdentities>>,
    runtime: RuntimeIdentity,
) -> Result<Arc<WorktreeSide>, RunnerError> {
    let base = root.join("wt");
    let (repo, checkouts) = (base.join("repo"), base.join("checkouts"));
    std::fs::create_dir_all(&repo).map_err(at("create the fixture repository"))?;
    std::fs::create_dir_all(&checkouts).map_err(at("create the checkout directory"))?;
    git(&repo, &["init", "-q", "-b", "main"])?;
    std::fs::write(repo.join("README"), "kept\n").map_err(at("write the repository file"))?;
    git(&repo, &["add", "README"])?;
    let identity = [
        "-c",
        "user.name=fixture",
        "-c",
        "user.email=fixture@invalid",
    ];
    git(&repo, &[&identity[..], &["commit", "-qm", "init"]].concat())?;
    let repository = Repository {
        id: serde_json::from_value(json!("repo:main")).map_err(at("name the repository"))?,
        path: repo,
        checkouts: checkouts.clone(),
    };
    let config = Config {
        root: base.clone(),
        git: Git::new(PathBuf::from("git"), Duration::from_secs(60)),
        repositories: vec![repository],
    };
    let service = Worktrees::new(config, Arc::new(Opened::default()))
        .map_err(at("open the worktree service"))?;
    let store = Store::open(&base.join("state.db"), StoreOptions::default())
        .map_err(at("open the worktree store"))?;
    let producer = OperationProducer::new(runtime.runtime_id());
    let core: Arc<Core> = Arc::new(Coordinator::new(store, runtime, producer));
    let mut dispatch = Dispatcher::new(Arc::clone(authority), runtime, Arc::clone(&core));
    Lane::new(Arc::new(service), core, runtime.machine_id())
        .register(&mut dispatch)
        .map_err(at("register worktree.*"))?;
    Ok(Arc::new(WorktreeSide {
        dispatch: Arc::new(dispatch),
        checkouts: std::fs::canonicalize(checkouts).map_err(at("resolve the checkouts"))?,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirtyParams {
    path: PathBuf,
}

impl WorktreeSide {
    /// `fixture.worktrees`: writes an unsaved note into a checkout this lane created.
    pub(crate) fn dirty(&self, request: &RpcRequest) -> Result<Value, BoundaryError> {
        let params: DirtyParams = decode_params(request).map_err(BoundaryError::from)?;
        let checkout = std::fs::canonicalize(&params.path)
            .map_err(|_| BoundaryError::invalid_params(Some("params.path")))?;
        if !checkout.starts_with(&self.checkouts) {
            return Err(BoundaryError::invalid_params(Some("params.path")));
        }
        std::fs::write(checkout.join("notes.txt"), "unsaved").map_err(internal)?;
        Ok(json!({"dirty": true}))
    }

    /// Joins the lane's operations; its store closes with the last handle.
    pub(crate) async fn shutdown(&self) -> Result<(), RunnerError> {
        self.dispatch
            .shutdown()
            .await
            .map_err(at("shut the worktree lane down"))
    }
}
