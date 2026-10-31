//! `worktree.remove`: enumerate, require explicit scope, close linked workspaces, then
//! remove the checkout; refusals change nothing and partial effects are reported.

use std::ffi::OsStr;

use ovis_terminal_protocol::domain::worktrees::AffectedResource;
use ovis_terminal_protocol::schema::methods::worktree::WorktreeRemoveParams;

use super::git::Invocation;
use super::scope::Exposure;
use super::service::{Observed, Session, record};
use super::{Call, Cancel, Change, WorktreeError, Worktrees};

impl Worktrees {
    /// Enumerates and admits a removal without changing anything.
    ///
    /// # Errors
    ///
    /// Returns the refusal `remove` would return, naming every affected resource.
    pub async fn check_remove(
        &self,
        call: Call<'_, WorktreeRemoveParams>,
    ) -> Result<(), WorktreeError> {
        let mut session = self.begin(call.cancel).await;
        self.plan_remove(&mut session, call.params).await.map(drop)
    }

    /// Removes a linked checkout once every needed scope was given.
    ///
    /// # Errors
    ///
    /// Returns refusals, Git failures, `cancelled` or `outcome_unknown` with committed effects.
    pub async fn remove(
        &self,
        call: Call<'_, WorktreeRemoveParams>,
    ) -> Result<Change, WorktreeError> {
        let mut session = self.begin(call.cancel).await;
        let (seen, exposure) = self.plan_remove(&mut session, call.params).await?;
        let mut last = record(known(&session, &seen)?, &seen)?;
        let closed = exposure.workspaces.clone();
        if !closed.is_empty() {
            self.workspaces
                .close(&closed)
                .map_err(WorktreeError::Workspaces)?;
            if let Some(known) = session.state.get_mut(&seen.id) {
                known.workspaces.clear();
            }
        }
        let mut committed = exposure.affected();
        committed.remove(0);
        let mut args: Vec<&OsStr> = vec!["worktree".as_ref(), "remove".as_ref()];
        args.extend(exposure.force_flags().iter().map(OsStr::new));
        args.extend(["--".as_ref(), seen.listed.path.as_os_str()]);
        let repository = known(&session, &seen)?.repository.clone();
        let dir = self.repository(&repository)?.path.clone();
        let call = Invocation {
            operation: "worktree remove",
            dir: &dir,
            args,
        };
        let removed = self.git.output(call, &mut session.cancel).await;
        if let Err(error) = removed {
            session.cancel = Cancel::never();
            let gone = self.observe(&mut session, &repository).await;
            let gone = gone.map(|live| live.iter().all(|other| other.id != seen.id));
            if matches!(gone, Ok(true)) {
                committed.insert(0, AffectedResource::Checkout(seen.id.clone()));
            }
            return Err(match (error, gone) {
                (WorktreeError::Interrupted { .. }, Ok(_)) => {
                    WorktreeError::Interrupted { committed }
                }
                (error, Ok(_)) if committed.is_empty() => error,
                (error, Ok(_)) => WorktreeError::Partial {
                    committed,
                    source: Box::new(error),
                },
                (_, Err(inspection)) => WorktreeError::Partial {
                    committed,
                    source: Box::new(inspection),
                },
            });
        }
        let retired = session.state.retire(&seen.id);
        last.workspaces.clear();
        last.revision = ovis_terminal_protocol::schema::scalar::Revision::new(
            retired.map_or(0, |known| known.revision).saturating_add(1),
        );
        drop(session);
        Ok(Change {
            record: last,
            workspaces: closed,
        })
    }

    async fn plan_remove(
        &self,
        session: &mut Session<'_>,
        params: &WorktreeRemoveParams,
    ) -> Result<(Observed, Exposure), WorktreeError> {
        let (seen, known) = self.find(session, &params.checkout_id).await?;
        if seen.primary {
            return Err(WorktreeError::PrimaryCheckout(seen.id));
        }
        self.inside(&seen.listed.path)?;
        let exposure = Exposure {
            checkout: seen.id.clone(),
            workspaces: known.workspaces,
            dirty: seen.dirty,
            locked: seen.listed.locked,
        };
        exposure.admit(params.scope)?;
        Ok((seen, exposure))
    }
}

fn known<'s>(
    session: &'s Session<'_>,
    seen: &Observed,
) -> Result<&'s super::state::Known, WorktreeError> {
    session
        .state
        .get(&seen.id)
        .ok_or_else(|| WorktreeError::UnknownCheckout(seen.id.clone()))
}
