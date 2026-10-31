//! `worktree.create` from an explicit source checkout and `worktree.open`; the created
//! checkout is reported apart from any workspace or program launched later.

use std::ffi::OsStr;
use std::path::PathBuf;

use ovis_terminal_protocol::domain::worktrees::{AffectedResource, BranchName};
use ovis_terminal_protocol::schema::ids::CheckoutId;
use ovis_terminal_protocol::schema::methods::worktree::{WorktreeCreateParams, WorktreeOpenParams};

use super::git::Invocation;
use super::service::{Session, record};
use super::{Call, Cancel, Change, WorktreeError, Worktrees};

/// A validated create request.
#[derive(Debug)]
struct Planned {
    source: CheckoutId,
    source_path: PathBuf,
    branch: BranchName,
    path: PathBuf,
}

impl Worktrees {
    /// Validates a create request without touching the repository.
    ///
    /// # Errors
    ///
    /// Returns the refusal `create` would return before running `git worktree add`.
    pub async fn check_create(
        &self,
        call: Call<'_, WorktreeCreateParams>,
    ) -> Result<(), WorktreeError> {
        let mut session = self.begin(call.cancel).await;
        self.plan_create(&mut session, call.params).await.map(drop)
    }

    /// Creates a branch and checkout from the source checkout's `HEAD`.
    ///
    /// # Errors
    ///
    /// Returns typed refusals, Git failures, or `cancelled` listing committed effects.
    pub async fn create(
        &self,
        call: Call<'_, WorktreeCreateParams>,
    ) -> Result<Change, WorktreeError> {
        let mut session = self.begin(call.cancel).await;
        let planned = self.plan_create(&mut session, call.params).await?;
        let branch = OsStr::new(planned.branch.as_str());
        let add = Invocation {
            operation: "worktree add",
            dir: &planned.source_path,
            args: vec![
                "worktree".as_ref(),
                "add".as_ref(),
                "-b".as_ref(),
                branch,
                "--".as_ref(),
                planned.path.as_os_str(),
                "HEAD".as_ref(),
            ],
        };
        match self.git.output(add, &mut session.cancel).await {
            Ok(_) => {}
            Err(WorktreeError::Interrupted { .. }) => {
                session.cancel = Cancel::never();
                return Err(match self.committed(&mut session, &planned).await {
                    Ok(committed) => WorktreeError::Interrupted { committed },
                    Err(error) => WorktreeError::Partial {
                        committed: Vec::new(),
                        source: Box::new(error),
                    },
                });
            }
            Err(error) => return Err(error),
        }
        let repository = call.params.repository_id.clone();
        let observed = self.observe(&mut session, &repository).await?;
        let created = std::fs::canonicalize(&planned.path).unwrap_or(planned.path);
        let seen = observed
            .into_iter()
            .find(|seen| seen.listed.path == created);
        let seen = seen.ok_or(WorktreeError::Vcs {
            operation: "worktree add",
            code: None,
        })?;
        let known = session
            .state
            .get_mut(&seen.id)
            .ok_or(WorktreeError::Porcelain)?;
        known.source = Some(planned.source);
        known.revision = known.revision.saturating_add(1);
        let record = record(known, &seen)?;
        drop(session);
        Ok(Change {
            record,
            workspaces: Vec::new(),
        })
    }

    /// Validates an open request without opening a workspace.
    ///
    /// # Errors
    ///
    /// Returns `not_found` or `path_forbidden`.
    pub async fn check_open(
        &self,
        call: Call<'_, WorktreeOpenParams>,
    ) -> Result<(), WorktreeError> {
        let mut session = self.begin(call.cancel).await;
        let (seen, _) = self.find(&mut session, &call.params.checkout_id).await?;
        drop(session);
        self.inside(&seen.listed.path)
    }

    /// Opens one workspace on an explicit checkout and links it.
    ///
    /// # Errors
    ///
    /// Returns `not_found`, `path_forbidden`, a Git failure or `unavailable`.
    pub async fn open(&self, call: Call<'_, WorktreeOpenParams>) -> Result<Change, WorktreeError> {
        let mut session = self.begin(call.cancel).await;
        let checkout = &call.params.checkout_id;
        let (seen, _) = self.find(&mut session, checkout).await?;
        self.inside(&seen.listed.path)?;
        let opened = self.workspaces.open(checkout, &seen.listed.path);
        let workspace = opened.map_err(WorktreeError::Workspaces)?;
        let missing = || WorktreeError::UnknownCheckout(checkout.clone());
        let known = session.state.get_mut(checkout).ok_or_else(missing)?;
        known.workspaces.push(workspace);
        known.revision = known.revision.saturating_add(1);
        let record = record(known, &seen)?;
        drop(session);
        Ok(Change {
            record,
            workspaces: vec![workspace],
        })
    }

    async fn plan_create(
        &self,
        session: &mut Session<'_>,
        params: &WorktreeCreateParams,
    ) -> Result<Planned, WorktreeError> {
        let repository = self.repository(&params.repository_id)?;
        let checkouts = repository.checkouts.clone();
        let branch = BranchName::parse(&params.branch)?;
        let (seen, known) = self.find(session, &params.source).await?;
        if known.repository != params.repository_id {
            return Err(WorktreeError::UnknownCheckout(params.source.clone()));
        }
        let format = Invocation {
            operation: "check-ref-format",
            dir: &seen.listed.path,
            args: vec![
                "check-ref-format".as_ref(),
                "--branch".as_ref(),
                branch.as_str().as_ref(),
            ],
        };
        if !self.git.run(format, &mut session.cancel).await?.success {
            return Err(WorktreeError::RefFormat);
        }
        if self
            .branch_exists(session, (&seen.listed.path, &branch))
            .await?
        {
            return Err(WorktreeError::Exists {
                affected: vec![AffectedResource::Branch(branch)],
            });
        }
        let path = checkouts.join(branch.as_str().replace('/', "-"));
        if path.exists() {
            return Err(WorktreeError::Exists {
                affected: Vec::new(),
            });
        }
        self.inside(&path)?;
        Ok(Planned {
            source: params.source.clone(),
            source_path: seen.listed.path,
            branch,
            path,
        })
    }

    async fn branch_exists(
        &self,
        session: &mut Session<'_>,
        (dir, branch): (&std::path::Path, &BranchName),
    ) -> Result<bool, WorktreeError> {
        let reference = format!("refs/heads/{branch}");
        let verify = Invocation {
            operation: "rev-parse",
            dir,
            args: vec![
                "rev-parse".as_ref(),
                "--verify".as_ref(),
                "--quiet".as_ref(),
                reference.as_ref(),
            ],
        };
        Ok(self.git.run(verify, &mut session.cancel).await?.success)
    }

    /// What an interrupted `worktree add` left behind.
    async fn committed(
        &self,
        session: &mut Session<'_>,
        planned: &Planned,
    ) -> Result<Vec<AffectedResource>, WorktreeError> {
        let mut committed = Vec::new();
        if self
            .branch_exists(session, (&planned.source_path, &planned.branch))
            .await?
        {
            committed.push(AffectedResource::Branch(planned.branch.clone()));
        }
        let repository = session
            .state
            .get(&planned.source)
            .map(|known| known.repository.clone());
        let repository =
            repository.ok_or_else(|| WorktreeError::UnknownCheckout(planned.source.clone()))?;
        let created = std::fs::canonicalize(&planned.path).unwrap_or_else(|_| planned.path.clone());
        for seen in self.observe(session, &repository).await? {
            if seen.listed.path == created
                && let Some(known) = session.state.get_mut(&seen.id)
            {
                known.source = Some(planned.source.clone());
                known.revision = known.revision.saturating_add(1);
                committed.push(AffectedResource::Checkout(seen.id));
            }
        }
        Ok(committed)
    }
}
