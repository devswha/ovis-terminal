//! A real dispatcher, store and socket serving the worktree lane.
//!
//! The Git repository is private to the test; workspaces are recorded by a test owner.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ovis_terminal_core::events::{Coordinator, EventData};
use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
use ovis_terminal_core::registry::WorkspaceId;
use ovis_terminal_protocol::schema::ids::CheckoutId;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_server::dispatch::Dispatcher;
use ovis_terminal_server::dispatch::lanes::worktrees::Lane;
use ovis_terminal_server::listen::ServerHandle;
use ovis_terminal_services::worktrees::{
    Config, Git, Repository, WorkspaceFailure, Workspaces, WorktreeEvent, Worktrees,
};
use ovis_terminal_store::{Store, StoreOptions};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::support::socket::{self, Client};
use super::support::{TestResult, runtime};

/// Operation and worktree events on one coordinator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum Event {
    Worktree(WorktreeEvent),
    Operation(Box<OperationEvent>),
}

impl From<OperationEvent> for Event {
    fn from(event: OperationEvent) -> Self {
        Self::Operation(Box::new(event))
    }
}

impl From<WorktreeEvent> for Event {
    fn from(event: WorktreeEvent) -> Self {
        Self::Worktree(event)
    }
}

impl EventData for Event {
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

#[derive(Debug, Default)]
pub(crate) struct Recorded {
    pub closed: Mutex<Vec<WorkspaceId>>,
    opened: Mutex<u64>,
}

impl Workspaces for Recorded {
    fn open(&self, _: &CheckoutId, _: &Path) -> Result<WorkspaceId, WorkspaceFailure> {
        let mut opened = self.opened.lock().map_err(|_| "poisoned")?;
        *opened = opened.checked_add(1).ok_or("overflow")?;
        let id = WorkspaceId::try_from(*opened)?;
        drop(opened);
        Ok(id)
    }

    fn close(&self, workspaces: &[WorkspaceId]) -> Result<(), WorkspaceFailure> {
        self.closed
            .lock()
            .map_err(|_| "poisoned")?
            .extend(workspaces);
        Ok(())
    }
}

pub(crate) type Core = Coordinator<Event, OperationProducer>;

pub(crate) struct Fixture {
    pub socket: socket::Fixture,
    pub repo: PathBuf,
    pub core: Arc<Core>,
    pub dispatch: Arc<Dispatcher<Event, OperationProducer>>,
    pub workspaces: Arc<Recorded>,
}

pub(crate) fn git(dir: &Path, args: &[&str]) -> TestResult<String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir).args(args);
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!("git {args:?}: {}", String::from_utf8_lossy(&output.stderr)).into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

impl Fixture {
    pub(crate) fn new(program: PathBuf) -> TestResult<Self> {
        let socket = socket::Fixture::new()?;
        let root = socket.path("root");
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo)?;
        std::fs::create_dir_all(root.join("checkouts"))?;
        git(&repo, &["init", "-q", "-b", "main"])?;
        std::fs::write(repo.join("README"), "kept\n")?;
        git(&repo, &["add", "README"])?;
        git(
            &repo,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@e",
                "commit",
                "-qm",
                "init",
            ],
        )?;
        let workspaces = Arc::new(Recorded::default());
        let service = Worktrees::new(
            Config {
                root: root.clone(),
                git: Git::new(program, Duration::from_secs(60)),
                repositories: vec![Repository {
                    id: serde_json::from_value(json!("repo:main"))?,
                    path: repo.clone(),
                    checkouts: root.join("checkouts"),
                }],
            },
            Arc::clone(&workspaces) as Arc<dyn Workspaces>,
        )?;
        let identity = runtime(1)?;
        let store = Store::open(&socket.path("db"), StoreOptions::default())?;
        let producer = OperationProducer::new(identity.runtime_id());
        let core = Arc::new(Coordinator::new(store, identity, producer));
        let mut dispatch =
            Dispatcher::new(Arc::clone(&socket.authority), identity, Arc::clone(&core));
        Lane::new(Arc::new(service), Arc::clone(&core), identity.machine_id())
            .register(&mut dispatch)?;
        Ok(Self {
            socket,
            repo: std::fs::canonicalize(repo)?,
            core,
            dispatch: Arc::new(dispatch),
            workspaces,
        })
    }

    pub(crate) async fn connect(&self) -> TestResult<(ServerHandle, Client)> {
        let server = self.socket.serve(1, Arc::clone(&self.dispatch)).await?;
        let mut client = self.socket.connect().await?;
        client.hello("1", &self.socket.bearer).await?;
        Ok((server, client))
    }

    pub(crate) async fn close(self, server: ServerHandle) -> TestResult {
        server.shutdown().await?;
        self.dispatch.shutdown().await?;
        drop(self.dispatch);
        let core = Arc::try_unwrap(self.core).map_err(|_| "core still owned")?;
        tokio::task::spawn_blocking(move || core.close()).await??;
        Ok(())
    }
}
