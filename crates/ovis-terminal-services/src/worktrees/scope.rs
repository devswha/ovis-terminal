//! Destructive scope: force covers dirty or locked content, group covers linked
//! workspaces, and neither scope ever stands in for the other.

use ovis_terminal_core::registry::WorkspaceId;
use ovis_terminal_protocol::domain::worktrees::AffectedResource;
use ovis_terminal_protocol::schema::ids::{self, CheckoutId};
use ovis_terminal_protocol::schema::methods::worktree::RemoveScope;

use super::WorktreeError;

/// What removing one checkout would touch.
#[derive(Debug, Clone)]
pub(super) struct Exposure {
    pub checkout: CheckoutId,
    pub workspaces: Vec<WorkspaceId>,
    pub dirty: bool,
    pub locked: bool,
}

impl Exposure {
    /// The checkout and every linked workspace, in that order.
    pub(super) fn affected(&self) -> Vec<AffectedResource> {
        let mut affected = vec![AffectedResource::Checkout(self.checkout.clone())];
        affected.extend(self.workspaces.iter().map(|workspace| {
            AffectedResource::Workspace(ids::WorkspaceId::new(workspace.to_string()))
        }));
        affected
    }

    /// Admits removal only when every needed scope was given explicitly.
    pub(super) fn admit(&self, scope: RemoveScope) -> Result<(), WorktreeError> {
        let affected = self.affected();
        if self.locked && !scope.force {
            return Err(WorktreeError::Locked { affected });
        }
        if self.dirty && !scope.force {
            return Err(WorktreeError::Dirty { affected });
        }
        if !self.workspaces.is_empty() && !scope.group {
            return Err(WorktreeError::ScopeRequired { affected });
        }
        Ok(())
    }

    /// Git needs one `--force` for changes and a second for a lock.
    pub(super) const fn force_flags(&self) -> &'static [&'static str] {
        match (self.locked, self.dirty) {
            (true, _) => &["--force", "--force"],
            (false, true) => &["--force"],
            (false, false) => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use ovis_terminal_core::registry::WorkspaceId;
    use ovis_terminal_protocol::schema::ids::CheckoutId;
    use ovis_terminal_protocol::schema::methods::worktree::RemoveScope;

    use super::Exposure;
    use crate::worktrees::WorktreeError;

    #[test]
    fn each_scope_covers_only_its_own_exposure() -> Result<(), Box<dyn std::error::Error>> {
        let linked = vec![WorkspaceId::try_from(4)?];
        for bits in 0_u8..32 {
            let [dirty, locked, has_group, group, force] =
                [1_u8, 2, 4, 8, 16].map(|mask| bits & mask != 0);
            let exposure = Exposure {
                checkout: CheckoutId::new("checkout:a".to_owned()),
                workspaces: if has_group {
                    linked.clone()
                } else {
                    Vec::new()
                },
                dirty,
                locked,
            };
            let admitted = exposure.admit(RemoveScope::new(group, force));
            let expected = if locked && !force {
                Some("checkout_locked")
            } else if dirty && !force {
                Some("checkout_dirty")
            } else if has_group && !group {
                Some("scope_required")
            } else {
                None
            };
            assert_eq!(
                admitted.as_ref().err().map(WorktreeError::kind),
                expected,
                "{bits:05b}"
            );
            if let Err(refusal) = admitted {
                assert_eq!(refusal.affected(), exposure.affected().as_slice());
                assert_eq!(refusal.affected().len(), 1 + usize::from(has_group));
            }
        }
        Ok(())
    }
}
