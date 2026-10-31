//! The worktree service: registered repositories inside one authorized root, Git
//! observation and the identity state, serialized per service.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ovis_terminal_protocol::domain::worktrees::{BranchName, WorktreeFacts};
use ovis_terminal_protocol::schema::ids::{self, CheckoutId, RepositoryId};
use ovis_terminal_protocol::schema::methods::worktree::{WorktreeListParams, WorktreeRecord};
use ovis_terminal_protocol::schema::scalar::Revision;
use tokio::sync::{Mutex, MutexGuard};

use super::git::Invocation;
use super::porcelain::{Listed, parse};
use super::state::{Known, Sighting, State};
use super::{Cancel, Git, Workspaces, WorktreeError, marker};

/// One repository: its primary checkout and the directory new checkouts go to.
#[derive(Debug, Clone)]
#[expect(
    clippy::exhaustive_structs,
    reason = "assembly registers repositories from configuration in another crate"
)]
pub struct Repository {
    /// Stable identity clients name.
    pub id: RepositoryId,
    /// Primary checkout.
    pub path: PathBuf,
    /// Directory that receives created checkouts.
    pub checkouts: PathBuf,
}

/// The authorized root, the Git executable and the registered repositories.
#[derive(Debug, Clone)]
#[expect(
    clippy::exhaustive_structs,
    reason = "assembly builds the configuration in another crate"
)]
pub struct Config {
    /// Every checkout path the service touches lies below this directory.
    pub root: PathBuf,
    /// Git invocation.
    pub git: Git,
    /// Registered repositories.
    pub repositories: Vec<Repository>,
}

/// One request and the cancellation of the operation that carries it.
#[derive(Debug)]
#[expect(
    clippy::exhaustive_structs,
    reason = "dispatch lanes in another crate pair parameters with their operation's cancellation"
)]
pub struct Call<'a, P> {
    /// The method's closed parameters.
    pub params: &'a P,
    /// Cancellation of the owning operation.
    pub cancel: Cancel,
}

/// Explicit-identity worktree service; mutations are serialized.
#[derive(Debug)]
pub struct Worktrees {
    pub(super) root: PathBuf,
    pub(super) git: Git,
    repositories: BTreeMap<RepositoryId, Repository>,
    pub(super) workspaces: Arc<dyn Workspaces>,
    state: Mutex<State>,
}

/// A checkout as Git currently lists it, with its identity.
#[derive(Debug, Clone)]
pub(super) struct Observed {
    pub id: CheckoutId,
    pub listed: Listed,
    pub dirty: bool,
    pub primary: bool,
}

/// The state lock and the cancellation of one request.
pub(super) struct Session<'a> {
    pub state: MutexGuard<'a, State>,
    pub cancel: Cancel,
}

fn canonical(path: &Path) -> Result<PathBuf, WorktreeError> {
    std::fs::canonicalize(path).map_err(|source| WorktreeError::Path {
        path: path.to_path_buf(),
        source,
    })
}

impl Worktrees {
    /// Registers repositories whose paths resolve inside the authorized root.
    ///
    /// # Errors
    ///
    /// Returns `path_not_found` for unresolved paths and `path_forbidden` outside the root.
    pub fn new(config: Config, workspaces: Arc<dyn Workspaces>) -> Result<Self, WorktreeError> {
        let root = canonical(&config.root)?;
        let mut repositories = BTreeMap::new();
        for repository in config.repositories {
            let resolved = Repository {
                path: canonical(&repository.path)?,
                checkouts: canonical(&repository.checkouts)?,
                id: repository.id,
            };
            for path in [&resolved.path, &resolved.checkouts] {
                if !path.starts_with(&root) {
                    return Err(WorktreeError::PathForbidden { path: path.clone() });
                }
            }
            repositories.insert(resolved.id.clone(), resolved);
        }
        Ok(Self {
            root,
            git: config.git,
            repositories,
            workspaces,
            state: Mutex::new(State::default()),
        })
    }

    /// Lists a repository's checkouts with branch, source, dirty, locked and workspaces.
    ///
    /// # Errors
    ///
    /// Returns `not_found` for an unknown repository or a typed Git failure.
    pub async fn list(
        &self,
        call: Call<'_, WorktreeListParams>,
    ) -> Result<Vec<WorktreeRecord>, WorktreeError> {
        let mut session = self.begin(call.cancel).await;
        let observed = self
            .observe(&mut session, &call.params.repository_id)
            .await?;
        let records = observed
            .iter()
            .map(|seen| {
                let known = session.state.get(&seen.id);
                record(
                    known.ok_or_else(|| WorktreeError::UnknownCheckout(seen.id.clone()))?,
                    seen,
                )
            })
            .collect();
        drop(session);
        records
    }

    pub(super) async fn begin(&self, cancel: Cancel) -> Session<'_> {
        Session {
            state: self.state.lock().await,
            cancel,
        }
    }

    pub(super) fn repository(&self, id: &RepositoryId) -> Result<&Repository, WorktreeError> {
        self.repositories
            .get(id)
            .ok_or_else(|| WorktreeError::UnknownRepository(id.clone()))
    }

    pub(super) fn inside(&self, path: &Path) -> Result<(), WorktreeError> {
        if path.starts_with(&self.root) {
            Ok(())
        } else {
            Err(WorktreeError::PathForbidden {
                path: path.to_path_buf(),
            })
        }
    }

    /// Lists the repository through Git, minting and retiring identities to match.
    pub(super) async fn observe(
        &self,
        session: &mut Session<'_>,
        repository: &RepositoryId,
    ) -> Result<Vec<Observed>, WorktreeError> {
        let repo = self.repository(repository)?;
        let call = Invocation {
            operation: "worktree list",
            dir: &repo.path,
            args: ["worktree", "list", "--porcelain", "-z"]
                .map(AsRef::as_ref)
                .to_vec(),
        };
        let listed = parse(&self.git.output(call, &mut session.cancel).await?)?;
        let admins = self.admins(session, repo).await?;
        let mut observed = Vec::with_capacity(listed.len());
        for (index, mut listed) in listed.into_iter().enumerate() {
            listed.path = std::fs::canonicalize(&listed.path).unwrap_or(listed.path);
            let primary = index == 0;
            let marker = marker::read_or_mint(&admins.of(&listed.path, primary)?)?;
            let dirty = if listed.bare || listed.prunable {
                false
            } else {
                self.dirty(session, &listed.path).await?
            };
            session.state.sight(Sighting {
                checkout: &marker.checkout_id,
                repository,
                path: listed.path.clone(),
                source: marker.source,
            });
            observed.push(Observed {
                id: marker.checkout_id,
                listed,
                dirty,
                primary,
            });
        }
        let live: Vec<CheckoutId> = observed.iter().map(|seen| seen.id.clone()).collect();
        session.state.retain(repository, &live);
        Ok(observed)
    }

    async fn dirty(&self, session: &mut Session<'_>, path: &Path) -> Result<bool, WorktreeError> {
        let call = Invocation {
            operation: "status",
            dir: path,
            args: ["status", "--porcelain=v1", "-z", "--untracked-files=normal"]
                .map(AsRef::as_ref)
                .to_vec(),
        };
        Ok(!self.git.output(call, &mut session.cancel).await?.is_empty())
    }

    /// The live checkout behind `checkout` and what the service knows about it.
    pub(super) async fn find(
        &self,
        session: &mut Session<'_>,
        checkout: &CheckoutId,
    ) -> Result<(Observed, Known), WorktreeError> {
        let missing = || WorktreeError::UnknownCheckout(checkout.clone());
        let repository = session
            .state
            .get(checkout)
            .ok_or_else(missing)?
            .repository
            .clone();
        let observed = self.observe(session, &repository).await?;
        let seen = observed.into_iter().find(|seen| seen.id == *checkout);
        let seen = seen.ok_or_else(missing)?;
        let known = session.state.get(checkout).ok_or_else(missing)?.clone();
        Ok((seen, known))
    }
}

/// The wire record of one observed checkout.
pub(super) fn record(known: &Known, seen: &Observed) -> Result<WorktreeRecord, WorktreeError> {
    let path = seen
        .listed
        .path
        .to_str()
        .ok_or_else(|| WorktreeError::PathForbidden {
            path: seen.listed.path.clone(),
        })?;
    let branch = seen.listed.branch.as_deref().map(BranchName::parse);
    Ok(WorktreeRecord::from(WorktreeFacts {
        repository_id: known.repository.clone(),
        checkout_id: seen.id.clone(),
        path: path.to_owned(),
        branch: branch.transpose().map_err(|_| WorktreeError::Porcelain)?,
        source: known.source.clone(),
        dirty: seen.dirty,
        locked: seen.listed.locked,
        workspaces: known
            .workspaces
            .iter()
            .map(|workspace| ids::WorkspaceId::new(workspace.to_string()))
            .collect(),
        revision: Revision::new(known.revision),
    }))
}
