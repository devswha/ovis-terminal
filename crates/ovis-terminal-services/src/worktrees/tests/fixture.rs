use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ovis_terminal_core::registry::WorkspaceId;
use ovis_terminal_protocol::domain::worktrees::AffectedResource;
use ovis_terminal_protocol::schema::ids::CheckoutId;
use ovis_terminal_protocol::schema::methods::worktree::WorktreeRecord;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::worktrees::{
    Call, Cancel, Config, Git, Repository, WorkspaceFailure, Workspaces, WorktreeError, Worktrees,
};

pub(super) type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// Records the workspaces the service opens and closes.
#[derive(Debug, Default)]
pub(super) struct Recorded {
    pub opened: Mutex<Vec<(CheckoutId, WorkspaceId)>>,
    pub closed: Mutex<Vec<WorkspaceId>>,
}

impl Workspaces for Recorded {
    fn open(&self, checkout: &CheckoutId, _: &Path) -> Result<WorkspaceId, WorkspaceFailure> {
        let mut opened = self.opened.lock().map_err(|_| "poisoned")?;
        let next = u64::try_from(opened.len())?
            .checked_add(1)
            .ok_or("overflow")?;
        let id = WorkspaceId::try_from(next)?;
        opened.push((checkout.clone(), id));
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

pub(super) struct Fixture {
    pub root: PathBuf,
    pub repo: PathBuf,
    pub service: Worktrees,
    pub workspaces: Arc<Recorded>,
    pub cancel: watch::Sender<bool>,
}

pub(super) fn git(dir: &Path, args: &[&str]) -> TestResult<String> {
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
    pub(super) fn new() -> TestResult<Self> {
        Self::with_git(PathBuf::from("git"), Duration::from_secs(30))
    }

    pub(super) fn with_git(program: PathBuf, bound: Duration) -> TestResult<Self> {
        let root = std::env::temp_dir().join(format!("ovt45-{}", uuid::Uuid::now_v7()));
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
        let config = Config {
            root: root.clone(),
            git: Git::new(program, bound),
            repositories: vec![Repository {
                id: serde_json::from_value(json!("repo:main"))?,
                path: repo,
                checkouts: root.join("checkouts"),
            }],
        };
        let service = Worktrees::new(config, Arc::clone(&workspaces) as Arc<dyn Workspaces>)?;
        let root = std::fs::canonicalize(root)?;
        let repo = root.join("repo");
        Ok(Self {
            root,
            repo,
            service,
            workspaces,
            cancel: watch::channel(false).0,
        })
    }

    pub(super) fn call<'p, P>(&self, params: &'p P) -> Call<'p, P> {
        Call {
            params,
            cancel: Cancel::new(self.cancel.subscribe()),
        }
    }

    pub(super) async fn list(&self) -> TestResult<Vec<WorktreeRecord>> {
        let params = params(json!({"repository_id": "repo:main"}))?;
        Ok(self.service.list(self.call(&params)).await?)
    }

    pub(super) async fn primary(&self) -> TestResult<CheckoutId> {
        let listed = self.list().await?;
        Ok(listed
            .first()
            .ok_or("no primary checkout")?
            .checkout_id
            .clone())
    }

    pub(super) async fn create(&self, branch: &str) -> Result<WorktreeRecord, Box<dyn Error>> {
        let params = params(json!({"op_id": "op", "repository_id": "repo:main",
            "source": self.primary().await?, "branch": branch}))?;
        Ok(self.service.create(self.call(&params)).await?.record)
    }

    pub(super) async fn remove(
        &self,
        checkout: &CheckoutId,
        (group, force): (bool, bool),
    ) -> TestResult<Result<crate::worktrees::Change, WorktreeError>> {
        let params = params(json!({"op_id": "op", "checkout_id": checkout,
            "scope": {"group": group, "force": force}}))?;
        Ok(self.service.remove(self.call(&params)).await)
    }
}

pub(super) fn params<P: DeserializeOwned>(value: Value) -> TestResult<P> {
    Ok(serde_json::from_value(value)?)
}

pub(super) fn targets(error: &WorktreeError) -> Vec<String> {
    let ids = error.affected().iter().map(AffectedResource::target_id);
    ids.map(|id| {
        serde_json::to_value(id)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default()
    })
    .collect()
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.root);
        assert!(removed.is_ok(), "cleanup: {removed:?}");
    }
}

pub(super) fn nth<T>(items: &[T], index: usize) -> TestResult<&T> {
    Ok(items.get(index).ok_or_else(|| format!("no item {index}"))?)
}
