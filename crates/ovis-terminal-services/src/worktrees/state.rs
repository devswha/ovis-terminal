//! Checkout identities minted on first sight and retired when Git stops listing them,
//! so a later checkout at the same path never inherits an old identity.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ovis_terminal_core::registry::WorkspaceId;
use ovis_terminal_protocol::schema::ids::{CheckoutId, RepositoryId};

/// What the service knows about one checkout beyond Git's listing.
#[derive(Debug, Clone)]
pub(super) struct Known {
    pub repository: RepositoryId,
    pub path: PathBuf,
    pub source: Option<CheckoutId>,
    pub workspaces: Vec<WorkspaceId>,
    pub revision: u64,
}

#[derive(Debug, Default)]
pub(super) struct State {
    checkouts: BTreeMap<CheckoutId, Known>,
}

impl State {
    pub(super) fn get(&self, checkout: &CheckoutId) -> Option<&Known> {
        self.checkouts.get(checkout)
    }

    pub(super) fn get_mut(&mut self, checkout: &CheckoutId) -> Option<&mut Known> {
        self.checkouts.get_mut(checkout)
    }

    /// The identity of `path` in `repository`, minting one on first sight.
    pub(super) fn identify(&mut self, repository: &RepositoryId, path: &Path) -> CheckoutId {
        let known = self
            .checkouts
            .iter()
            .find(|(_, known)| known.repository == *repository && known.path == path);
        if let Some((id, _)) = known {
            return id.clone();
        }
        let id = CheckoutId::new(format!("checkout:{}", uuid::Uuid::now_v7()));
        self.checkouts.insert(
            id.clone(),
            Known {
                repository: repository.clone(),
                path: path.to_path_buf(),
                source: None,
                workspaces: Vec::new(),
                revision: 0,
            },
        );
        id
    }

    /// Retires identities of `repository` whose paths Git no longer lists.
    pub(super) fn retain(&mut self, repository: &RepositoryId, live: &[PathBuf]) {
        self.checkouts
            .retain(|_, known| known.repository != *repository || live.contains(&known.path));
    }

    pub(super) fn retire(&mut self, checkout: &CheckoutId) -> Option<Known> {
        self.checkouts.remove(checkout)
    }
}
