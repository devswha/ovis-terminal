//! What the service holds per checkout identity beyond Git's listing: the path last seen,
//! linked workspaces and a change revision. Identities come from markers, never paths.

use std::collections::BTreeMap;
use std::path::PathBuf;

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

/// One listing of a checkout under its marker identity.
#[derive(Debug)]
pub(super) struct Sighting<'a> {
    pub checkout: &'a CheckoutId,
    pub repository: &'a RepositoryId,
    pub path: PathBuf,
    pub source: Option<CheckoutId>,
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

    /// Records where the identity was seen; links and revision stay with the identity.
    pub(super) fn sight(&mut self, sighting: Sighting<'_>) {
        let known = self
            .checkouts
            .entry(sighting.checkout.clone())
            .or_insert_with(|| Known {
                repository: sighting.repository.clone(),
                path: sighting.path.clone(),
                source: None,
                workspaces: Vec::new(),
                revision: 0,
            });
        known.path = sighting.path;
        known.source = sighting.source;
    }

    /// Retires identities of `repository` that the latest listing no longer contains.
    pub(super) fn retain(&mut self, repository: &RepositoryId, live: &[CheckoutId]) {
        self.checkouts
            .retain(|id, known| known.repository != *repository || live.contains(id));
    }

    pub(super) fn retire(&mut self, checkout: &CheckoutId) -> Option<Known> {
        self.checkouts.remove(checkout)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ovis_terminal_core::registry::WorkspaceId;
    use ovis_terminal_protocol::schema::ids::{CheckoutId, RepositoryId};

    use super::{Sighting, State};

    #[test]
    fn identities_missing_from_a_listing_are_retired_with_their_links()
    -> Result<(), Box<dyn std::error::Error>> {
        let repo = RepositoryId::new("repo:a".into());
        let other = RepositoryId::new("repo:b".into());
        let [kept, gone, foreign] =
            ["checkout:1", "checkout:2", "checkout:3"].map(|id| CheckoutId::new(id.into()));
        let mut state = State::default();
        for (checkout, repository) in [(&kept, &repo), (&gone, &repo), (&foreign, &other)] {
            let path = PathBuf::from(format!("/w/{}", checkout.as_str()));
            state.sight(Sighting {
                checkout,
                repository,
                path,
                source: None,
            });
        }
        let linked = WorkspaceId::try_from(7)?;
        state.get_mut(&gone).ok_or("gone")?.workspaces.push(linked);
        state.retain(&repo, std::slice::from_ref(&kept));
        assert!(state.get(&kept).is_some());
        assert!(
            state.get(&gone).is_none(),
            "an unlisted identity keeps no links"
        );
        assert!(
            state.get(&foreign).is_some(),
            "another repository's listing is untouched"
        );
        let path = PathBuf::from("/w/checkout:2");
        state.sight(Sighting {
            checkout: &gone,
            repository: &repo,
            path,
            source: None,
        });
        let resighted = state.get(&gone).ok_or("re-sighted")?;
        assert_eq!(
            resighted.workspaces,
            [],
            "links never come back with the identity"
        );
        Ok(())
    }
}
