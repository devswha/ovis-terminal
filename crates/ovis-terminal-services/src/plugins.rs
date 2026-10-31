//! Scoped plugin catalogs and bounded, durable run outcomes (plan todo 46).
//!
//! Plugins live below one runtime-owned root, never a user-global directory. An action is
//! an absolute program plus argv template; it runs without a shell, with a cleared
//! environment, and settles from its exit status rather than from anything it prints.

mod catalog;
mod error;
mod input;
mod manifest;
mod process;
mod runs;

use std::ffi::OsString;
use std::path::PathBuf;

pub use catalog::Catalog;
pub use error::PluginError;
use ovis_terminal_protocol::domain::plugins::PluginContext;
use ovis_terminal_protocol::schema::ids::{PluginRunId, TerminalId};
use ovis_terminal_protocol::schema::methods::plugin::{
    PluginInvokeParams, PluginRunRecord, PluginRunState,
};
use process::Launch;
use runs::Runs;
use tokio::sync::watch;

#[cfg(all(test, unix))]
mod tests;

/// The plugin service of one runtime: its plugin root and the runs of this lifetime.
#[derive(Debug)]
pub struct Plugins {
    root: PathBuf,
    runs: Runs,
}

/// An invocation that passed availability, context and input checks; nothing ran yet.
#[derive(Debug)]
pub struct Prepared {
    run_id: PluginRunId,
    launch: Launch,
}

impl Prepared {
    /// The run this invocation will start.
    #[must_use]
    pub const fn run_id(&self) -> &PluginRunId {
        &self.run_id
    }
}

impl Plugins {
    /// Serves the plugins installed below `root`.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            runs: Runs::default(),
        }
    }

    /// Reads the current catalog; manifests are read fresh, so enabling applies at once.
    ///
    /// # Errors
    ///
    /// Returns the discovery errors of [`Catalog::discover`].
    pub fn catalog(&self) -> Result<Catalog, PluginError> {
        Catalog::discover(&self.root)
    }

    /// Checks an invocation against the current catalog without starting anything.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::NotFound`], [`PluginError::Disabled`],
    /// [`PluginError::Unavailable`] with the listed reason, [`PluginError::ContextUnsupported`]
    /// or [`PluginError::InvalidInput`]; discovery errors as for [`Plugins::catalog`].
    pub fn prepare(
        &self,
        run_id: PluginRunId,
        params: &PluginInvokeParams,
    ) -> Result<Prepared, PluginError> {
        let catalog = self.catalog()?;
        let (plugin, action) = catalog.find(&params.plugin_id, &params.action_id)?;
        if !plugin.enabled {
            return Err(PluginError::Disabled);
        }
        if let Some(reason) = action.unavailable {
            return Err(PluginError::Unavailable { reason });
        }
        if params.target.is_some() || !action.contexts.contains(&PluginContext::Machine) {
            return Err(PluginError::ContextUnsupported);
        }
        let definition = action.definition.as_ref().ok_or(PluginError::Unavailable {
            reason: ovis_terminal_protocol::domain::plugins::Unavailable::DefinitionInvalid,
        })?;
        let expanded = definition.expand(&params.input)?;
        let mut env = Launch::inherited_env();
        env.extend([
            (
                "OVIS_PLUGIN_RUN_ID".to_owned(),
                OsString::from(run_id.as_str()),
            ),
            (
                "OVIS_PLUGIN_ACTION".to_owned(),
                OsString::from(params.action_id.as_str()),
            ),
            (
                "OVIS_PLUGIN_INPUT".to_owned(),
                OsString::from(expanded.input.to_string()),
            ),
        ]);
        Ok(Prepared {
            run_id,
            launch: Launch {
                program: expanded.program,
                args: expanded.args,
                cwd: plugin.directory.clone(),
                env,
            },
        })
    }

    /// Starts the prepared program and supervises it to its outcome.
    ///
    /// `cancel` turning `true` kills the program and settles the run as cancelled.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::SpawnFailed`] when the program could not start, I/O errors
    /// while supervising it, [`PluginError::RunExists`] or [`PluginError::RunCapacity`].
    pub async fn run(
        &self,
        prepared: Prepared,
        cancel: watch::Receiver<bool>,
    ) -> Result<PluginRunRecord, PluginError> {
        let run = self.runs.start(prepared.run_id)?;
        match process::supervise(prepared.launch, &run, cancel).await {
            Ok(()) => run.record(),
            Err(error) => {
                run.settle(PluginRunState::Failed, None)?;
                Err(error)
            }
        }
    }

    /// The live record of a run of this lifetime, if it is still retained.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::Poisoned`] when the run table is unusable.
    pub fn live_record(
        &self,
        run_id: &PluginRunId,
    ) -> Result<Option<PluginRunRecord>, PluginError> {
        self.runs.get(run_id)?.map(|run| run.record()).transpose()
    }

    /// Links a terminal that runtime authorization created on behalf of a running run.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::NotFound`] when the run is unknown or already settled.
    pub fn link_created(
        &self,
        run_id: &PluginRunId,
        terminal: TerminalId,
    ) -> Result<(), PluginError> {
        self.runs.link_created(run_id, terminal)
    }
}
