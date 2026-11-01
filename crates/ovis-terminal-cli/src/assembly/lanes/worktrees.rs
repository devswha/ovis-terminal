//! `worktree.*` over the worktree service, committing to the dispatch log. Repositories
//! are registered in `<state>/worktrees.json`; without it the root is `<state>/worktrees`
//! and none is registered. Workspaces a checkout opens are layout workspaces the runtime
//! creates and closes under the control's own session.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ovis_terminal_core::registry::WorkspaceId;
use ovis_terminal_protocol::schema::ids::CheckoutId;
use ovis_terminal_server::dispatch::lanes::worktrees::Lane;
use ovis_terminal_services::worktrees::{
    Config, Git, Repository, WorkspaceFailure, Workspaces, Worktrees,
};
use serde::Deserialize;

use super::{Dispatch, DispatchLog, Hosting, stage};
use crate::assembly::AssemblyError;
use crate::assembly::actor::Actor;
use crate::assembly::error::worker_gone;

/// Bound on one Git command.
const GIT_BOUND: Duration = Duration::from_secs(60);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registered {
    root: std::path::PathBuf,
    repositories: Vec<Registration>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    id: ovis_terminal_protocol::schema::ids::RepositoryId,
    path: std::path::PathBuf,
    checkouts: std::path::PathBuf,
}

/// Opens and closes layout workspaces on the runtime's actor.
#[derive(Debug)]
struct LayoutWorkspaces(Actor);

impl Workspaces for LayoutWorkspaces {
    fn open(&self, checkout: &CheckoutId, path: &Path) -> Result<WorkspaceId, WorkspaceFailure> {
        let name = path.file_name().map_or_else(
            || checkout.as_str().to_owned(),
            |name| name.to_string_lossy().into(),
        );
        let opened = self
            .0
            .call_blocking(move |runtime| runtime.open_workspace(&name));
        Ok(opened.ok_or_else(worker_gone)??)
    }

    fn close(&self, workspaces: &[WorkspaceId]) -> Result<(), WorkspaceFailure> {
        let workspaces = workspaces.to_vec();
        let closed = self
            .0
            .call_blocking(move |runtime| runtime.close_workspaces(&workspaces));
        Ok(closed.ok_or_else(worker_gone)??)
    }
}

fn config(state: &Path) -> Result<Config, AssemblyError> {
    let file = state.join("worktrees.json");
    let registered = if file.exists() {
        let bytes = std::fs::read(&file).map_err(stage("read worktrees.json"))?;
        serde_json::from_slice(&bytes).map_err(stage("parse worktrees.json"))?
    } else {
        let root = state.join("worktrees");
        std::fs::create_dir_all(&root).map_err(stage("create the worktree root"))?;
        Registered {
            root,
            repositories: Vec::new(),
        }
    };
    Ok(Config {
        root: registered.root,
        git: Git::new("git".into(), GIT_BOUND),
        repositories: registered
            .repositories
            .into_iter()
            .map(|repository| Repository {
                id: repository.id,
                path: repository.path,
                checkouts: repository.checkouts,
            })
            .collect(),
    })
}

/// Registers `worktree.list`, `create`, `open` and `remove`.
pub(super) fn register(
    dispatch: &mut Dispatch,
    hosting: &Hosting<'_>,
    log: &Arc<DispatchLog>,
) -> Result<(), AssemblyError> {
    let workspaces: Arc<dyn Workspaces> = Arc::new(LayoutWorkspaces(hosting.actor.clone()));
    let service = Worktrees::new(config(hosting.state)?, workspaces)
        .map_err(stage("open the worktree service"))?;
    Lane::new(
        Arc::new(service),
        Arc::clone(log),
        hosting.identity.machine_id(),
    )
    .register(dispatch)
    .map_err(stage("register worktree.*"))
}
