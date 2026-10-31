//! Checkout identity lives in an ovis marker inside the worktree's Git admin directory
//! (`<common>/worktrees/<name>`, or the common directory for the primary checkout).
//!
//! Git deletes that directory with the worktree, so a checkout re-created at the same
//! path has no marker and gets a new identity; a moved worktree keeps its directory and
//! its identity. The path is reported, never used as the key.

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use ovis_terminal_protocol::schema::ids::CheckoutId;
use serde::{Deserialize, Serialize};

use super::git::Invocation;
use super::service::{Observed, Session};
use super::{Repository, WorktreeError, Worktrees};

const MARKER: &str = "ovis-terminal-checkout.json";

/// The identity and source relationship recorded for one checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Marker {
    pub checkout_id: CheckoutId,
    pub source: Option<CheckoutId>,
}

/// Admin directories of a repository's checkouts, keyed by the checkout path they serve.
#[derive(Debug)]
pub(super) struct Admins {
    common: PathBuf,
    linked: BTreeMap<PathBuf, PathBuf>,
}

impl Admins {
    /// The admin directory of the listed checkout; the primary uses the common directory.
    pub(super) fn of(&self, path: &Path, primary: bool) -> Result<PathBuf, WorktreeError> {
        if primary {
            return Ok(self.common.clone());
        }
        self.linked
            .get(path)
            .cloned()
            .ok_or(WorktreeError::Porcelain)
    }
}

fn failed(path: &Path) -> impl FnOnce(std::io::Error) -> WorktreeError + '_ {
    move |source| WorktreeError::Marker {
        path: path.to_path_buf(),
        source,
    }
}

fn resolve(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).unwrap_or(path)
}

impl Worktrees {
    /// Reads `<common>/worktrees/*/gitdir` to map each linked checkout to its admin directory.
    pub(super) async fn admins(
        &self,
        session: &mut Session<'_>,
        repository: &Repository,
    ) -> Result<Admins, WorktreeError> {
        let call = Invocation {
            operation: "rev-parse",
            dir: &repository.path,
            args: ["rev-parse", "--path-format=absolute", "--git-common-dir"]
                .map(AsRef::as_ref)
                .to_vec(),
        };
        let out = self.git.output(call, &mut session.cancel).await?;
        let common = String::from_utf8(out).map_err(|_| WorktreeError::Porcelain)?;
        let common = resolve(PathBuf::from(common.trim_end()));
        let mut linked = BTreeMap::new();
        let worktrees = common.join("worktrees");
        let entries = match std::fs::read_dir(&worktrees) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(Admins { common, linked });
            }
            Err(error) => return Err(failed(&worktrees)(error)),
        };
        for entry in entries {
            let admin = entry.map_err(failed(&worktrees))?.path();
            let gitdir = std::fs::read_to_string(admin.join("gitdir")).map_err(failed(&admin))?;
            let dotgit = admin.join(gitdir.trim_end());
            let checkout = dotgit.parent().ok_or(WorktreeError::Porcelain)?;
            linked.insert(resolve(checkout.to_path_buf()), admin);
        }
        Ok(Admins { common, linked })
    }

    /// Records `source` as the checkout `seen` was created from, in its marker and in memory.
    pub(super) async fn adopt(
        &self,
        session: &mut Session<'_>,
        (seen, source): (&Observed, &CheckoutId),
    ) -> Result<(), WorktreeError> {
        let missing = || WorktreeError::UnknownCheckout(seen.id.clone());
        let repository = session
            .state
            .get(&seen.id)
            .ok_or_else(missing)?
            .repository
            .clone();
        let admins = self.admins(session, self.repository(&repository)?).await?;
        let marker = Marker {
            checkout_id: seen.id.clone(),
            source: Some(source.clone()),
        };
        write(&admins.of(&seen.listed.path, seen.primary)?, &marker)?;
        let known = session.state.get_mut(&seen.id).ok_or_else(missing)?;
        known.source = marker.source;
        known.revision = known.revision.saturating_add(1);
        Ok(())
    }
}

/// The checkout's marker, minting and writing a new identity when none exists yet.
pub(super) fn read_or_mint(admin: &Path) -> Result<Marker, WorktreeError> {
    let path = admin.join(MARKER);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| failed(&path)(std::io::Error::new(ErrorKind::InvalidData, error))),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let marker = Marker {
                checkout_id: CheckoutId::new(format!("checkout:{}", uuid::Uuid::now_v7())),
                source: None,
            };
            write(admin, &marker)?;
            Ok(marker)
        }
        Err(error) => Err(failed(&path)(error)),
    }
}

/// Replaces the marker atomically.
pub(super) fn write(admin: &Path, marker: &Marker) -> Result<(), WorktreeError> {
    let path = admin.join(MARKER);
    let staged = admin.join(format!("{MARKER}.{}", uuid::Uuid::now_v7()));
    let bytes = serde_json::to_vec(marker)
        .map_err(|error| failed(&path)(std::io::Error::new(ErrorKind::InvalidData, error)))?;
    std::fs::write(&staged, bytes).map_err(failed(&staged))?;
    std::fs::rename(&staged, &path).map_err(failed(&path))
}
