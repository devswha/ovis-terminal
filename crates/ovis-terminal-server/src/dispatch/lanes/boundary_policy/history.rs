//! `agent.history`: sessions listed from provider stores below one authorized root.
//!
//! A store is named by a relative path. Traversal is refused by the protocol type before
//! the filesystem is touched; a symlink that leads out of the root is refused when it is
//! resolved; and the directory is resolved again after it was read, so a store swapped
//! for a symlink in between is refused rather than reported. Entries are listed, never
//! parsed (EXCL-028), and symlinked entries are skipped, never followed.
//!
//! Pages are offsets into the newest-first listing taken at each read, so a provider
//! writing between pages can shift them. A directory swapped and restored within one
//! read is not detected; only entry names and times are ever disclosed.

use std::path::Path;
use std::time::SystemTime;

use ovis_terminal_authz::validation::{PathError, ScopedRoot};
use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::registry::MachineId;
use ovis_terminal_protocol::domain::boundary_policy::StoreRef;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::schema::common::{PageInfo, PageToken};
use ovis_terminal_protocol::schema::methods::agent::{AgentHistoryParams, AgentHistoryResult};
use ovis_terminal_protocol::schema::records::SessionRef;
use ovis_terminal_protocol::schema::scalar::Revision;
use ovis_terminal_store::Snapshot;
use serde_json::Value;

use crate::dispatch::{Access, DispatchError, Read};

/// Sessions per page when the caller names no limit.
const DEFAULT_LIMIT: u32 = 100;
/// Directory entries one listing examines at most; a larger store is reported truncated.
const MAX_ENTRIES: usize = 10_000;

/// `agent.history` over provider stores below `root`.
#[derive(Debug)]
pub struct AgentHistory {
    root: ScopedRoot,
    machine: MachineId,
}

impl AgentHistory {
    /// Lists stores below `root` on `machine`.
    #[must_use]
    pub const fn new(root: ScopedRoot, machine: MachineId) -> Self {
        Self { root, machine }
    }
}

fn path_refusal(error: &PathError) -> BoundaryError {
    let refused = BoundaryError::domain(ErrorKind::from(error.kind().to_owned()), false)
        .with_hint(error.hint());
    match error.field_path() {
        Some(field) => refused.with_field_path(field),
        None => refused,
    }
}

/// Regular files directly in `dir`, newest first, then by name; symlinks are skipped.
fn sessions(dir: &Path) -> Result<(Vec<(SystemTime, String)>, bool), std::io::Error> {
    let mut found = Vec::new();
    let mut listing = std::fs::read_dir(dir)?;
    for entry in listing.by_ref().take(MAX_ENTRIES) {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if !metadata.file_type().is_file() {
            continue;
        }
        if let Ok(name) = entry.file_name().into_string() {
            found.push((metadata.modified()?, name));
        }
    }
    let truncated = listing.next().is_some();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    Ok((found, truncated))
}

fn page_of(
    (found, truncated): (Vec<(SystemTime, String)>, bool),
    (params, source): (&AgentHistoryParams, &str),
    revision: Revision,
) -> Result<AgentHistoryResult, BoundaryError> {
    let page = params.page.as_ref();
    let skip = match page.and_then(|page| page.after.as_ref()) {
        Some(token) => token
            .as_str()
            .parse::<usize>()
            .map_err(|_| BoundaryError::invalid_params(Some("params.page.after")))?,
        None => 0,
    };
    let limit = page.and_then(|page| page.limit).unwrap_or(DEFAULT_LIMIT);
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let total = u32::try_from(found.len()).unwrap_or(u32::MAX);
    let end = skip.saturating_add(limit);
    let next = (end < found.len()).then(|| PageToken::new(end.to_string()));
    let listed = found
        .into_iter()
        .skip(skip)
        .take(limit)
        .map(|(_, name)| SessionRef::new(source.to_owned(), ("file".to_owned(), name)))
        .collect();
    let info = PageInfo::new(revision, total, next);
    let info = if truncated { info.cut_short() } else { info };
    Ok(AgentHistoryResult::new(listed, info))
}

impl Read for AgentHistory {
    const NAME: &'static str = "agent.history";
    type Params = AgentHistoryParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(Access::new(
            Action::Read,
            ResolvedTarget::new(self.machine, TargetResource::Machine),
        ))
    }

    fn read(&self, params: Self::Params, snapshot: &Snapshot) -> Result<Value, BoundaryError> {
        let (store, source) = match params.store.as_deref() {
            Some(text) => {
                let store = StoreRef::parse(text).map_err(|error| error.to_boundary())?;
                let resolved = self.root.resolve((StoreRef::FIELD, store.path()));
                (resolved, store.as_str().to_owned())
            }
            None => (self.root.itself(StoreRef::FIELD), String::new()),
        };
        let store = store.map_err(|error| path_refusal(&error))?;
        let unreadable = |source| {
            path_refusal(&PathError::Io {
                field: StoreRef::FIELD,
                source,
            })
        };
        let listed = sessions(store.path()).map_err(unreadable)?;
        store.recheck().map_err(|error| path_refusal(&error))?;
        let revision = Revision::new(snapshot.revision().get());
        let result = page_of(listed, (&params, &source), revision)?;
        serde_json::to_value(result).map_err(|_| {
            BoundaryError::domain(ErrorKind::from("internal".to_owned()), false)
                .with_hint("retry the read; the listing could not be encoded")
        })
    }
}
