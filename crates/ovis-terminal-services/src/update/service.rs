//! The update service: configuration, durable state, `update.query` and `update.stage`.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::sync::{Arc, Mutex, MutexGuard};

use ovis_terminal_protocol::schema::ids::UpdateId;
use ovis_terminal_protocol::schema::methods::update::{
    UpdateQueryResult, UpdateRecord, UpdateStageParams,
};

use super::config::{Config, seed};
use super::digest::{Interrupt, copy_hashed, hash_file};
use super::layout::{Layout, io, is_update_id, load, new_update_id, remove_tree, store};
use super::store::{Journal, Selection};
use super::{RestartControl, UpdateError};

#[derive(Debug)]
pub(super) struct State {
    pub(super) records: BTreeMap<UpdateId, UpdateRecord>,
    pub(super) selection: Selection,
    pub(super) pending: Option<Journal>,
}

/// Staged updates of one runtime install; every step is durable before it returns.
///
/// Steps block on file I/O and the readiness probe; call them from a blocking thread
/// of a Tokio runtime.
#[derive(Debug)]
pub struct Updates {
    pub(super) layout: Layout,
    pub(super) config: Config,
    pub(super) control: Arc<dyn RestartControl>,
    pub(super) state: Mutex<State>,
}

impl Updates {
    /// Opens the install root and finishes or reverts an interrupted activation.
    ///
    /// # Errors
    ///
    /// Install-root I/O, corrupt records, or a recovery that cannot restore a build.
    pub fn open(config: Config, control: Arc<dyn RestartControl>) -> Result<Self, UpdateError> {
        let layout = Layout::create(config.install_root.clone())?;
        let selection = if let Some(selection) = load::<Selection>(&layout.selection())? {
            selection
        } else {
            let selection = seed(&config)?;
            store(&layout.selection(), &selection)?;
            selection
        };
        let mut records = BTreeMap::new();
        for entry in fs::read_dir(layout.records()).map_err(io("list update records"))? {
            let path = entry.map_err(io("list update records"))?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
                && let Some(record) = load::<UpdateRecord>(&path)?
            {
                records.insert(record.update_id.clone(), record);
            }
        }
        let pending = load::<Journal>(&layout.journal())?;
        let updates = Self {
            state: Mutex::new(State {
                records,
                selection,
                pending,
            }),
            layout,
            config,
            control,
        };
        updates.recover()?;
        Ok(updates)
    }

    pub(super) fn lock(&self) -> Result<MutexGuard<'_, State>, UpdateError> {
        self.state.lock().map_err(|_| UpdateError::Poisoned)
    }
}

impl State {
    pub(super) fn record(&self, id: &UpdateId) -> Result<UpdateRecord, UpdateError> {
        self.records
            .get(id)
            .filter(|_| is_update_id(id))
            .cloned()
            .ok_or_else(|| UpdateError::NoStagedUpdate(id.clone()))
    }
}

impl Updates {
    pub(super) fn save(&self, state: &mut State, record: &UpdateRecord) -> Result<(), UpdateError> {
        store(&self.layout.record(&record.update_id), record)?;
        state
            .records
            .insert(record.update_id.clone(), record.clone());
        Ok(())
    }

    /// The active build as read from disk now, its rollback target and staged updates.
    ///
    /// # Errors
    ///
    /// Install-root I/O, or the control's failure to list live incarnations.
    pub fn query(&self) -> Result<UpdateQueryResult, UpdateError> {
        let state = self.lock()?;
        let mut active = state.selection.active.build.clone();
        active.binary_sha256 = hash_file(
            &state.selection.active.binary,
            self.config.limits.archive_bytes,
        )?;
        let previous = state
            .selection
            .previous
            .as_ref()
            .map(|selected| selected.build.clone());
        let pending = state.pending.as_ref().map(Journal::pending);
        let updates = state.records.values().cloned().collect();
        drop(state);
        let live = self.control.live_incarnations()?;
        Ok(UpdateQueryResult::new(
            (active, previous),
            pending,
            (live, updates),
        ))
    }

    /// Copies the named release into the install root if it hashes to the requested digest.
    ///
    /// The active build and selection are untouched; a refusal or cancellation removes
    /// every partial file.
    ///
    /// # Errors
    ///
    /// `incompatible` for another target, `transport_failed` for a missing archive,
    /// `too_large`, `integrity_failed`, `cancelled`, or install-root I/O.
    pub fn stage(
        &self,
        params: &UpdateStageParams,
        interrupt: &Interrupt,
    ) -> Result<UpdateRecord, UpdateError> {
        let mut state = self.lock()?;
        let requirement = &self.config.requirement;
        let target = params.target.as_ref().unwrap_or(&requirement.target);
        if *target != requirement.target {
            return Err(UpdateError::WrongTarget {
                requested: target.clone(),
                host: requirement.target.clone(),
            });
        }
        let name = format!("ovis-terminal-{}-{target}.tar", params.version);
        let path = self
            .config
            .releases
            .join(params.version.as_str())
            .join(name);
        let mut source = File::open(&path).map_err(|source| UpdateError::ReleaseMissing {
            path: path.clone(),
            source,
        })?;
        let id = new_update_id();
        let dir = self.layout.staging(&id);
        fs::create_dir_all(&dir).map_err(io("create a staging directory"))?;
        let partial = dir.join("archive.tar.partial");
        let limits = (self.config.limits.archive_bytes, interrupt);
        let actual = match copy_hashed(&mut source, &partial, limits) {
            Ok(actual) => actual,
            Err(error) => {
                remove_tree(&dir)?;
                return Err(error);
            }
        };
        if actual != params.sha256 {
            remove_tree(&dir)?;
            let expected = params.sha256.clone();
            return Err(UpdateError::DigestMismatch { expected, actual });
        }
        fs::rename(&partial, self.layout.archive(&id)).map_err(io("keep a staged release"))?;
        let record = UpdateRecord::staged(
            id,
            (params.version.clone(), target.clone(), actual),
            (
                state.selection.active.build.version.clone(),
                params.op_id.clone(),
            ),
        );
        self.save(&mut state, &record)?;
        drop(state);
        Ok(record)
    }
}
