//! Discovery of the plugins installed below one runtime-owned root.

use std::path::{Path, PathBuf};

use ovis_terminal_protocol::domain::plugins::{PluginContext, Unavailable};
use ovis_terminal_protocol::schema::ids::{PluginActionId, PluginId};
use ovis_terminal_protocol::schema::methods::plugin::{
    PluginAction, PluginListResult, PluginRecord,
};
use serde_json::Value;

use super::PluginError;
use super::input::Definition;
use super::manifest::{self, ActionManifest};

#[derive(Debug, Clone)]
pub(super) struct ListedAction {
    pub(super) action_id: PluginActionId,
    input_schema: Value,
    pub(super) contexts: Vec<PluginContext>,
    pub(super) definition: Option<Definition>,
    pub(super) unavailable: Option<Unavailable>,
}

#[derive(Debug, Clone)]
pub(super) struct InstalledPlugin {
    pub(super) plugin_id: PluginId,
    version: String,
    pub(super) enabled: bool,
    pub(super) directory: PathBuf,
    pub(super) actions: Vec<ListedAction>,
}

/// The installed plugins, each action annotated with why it cannot run, if it cannot.
#[derive(Debug, Clone)]
pub struct Catalog {
    plugins: Vec<InstalledPlugin>,
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file())
}

fn action(enabled: bool, declared: ActionManifest) -> ListedAction {
    let definition = Definition::parse(&declared.argv, &declared.input_schema)
        .filter(|_| !declared.contexts.is_empty());
    let unavailable = if !enabled {
        Some(Unavailable::PluginDisabled)
    } else if !declared.runs_here() {
        Some(Unavailable::PlatformUnsupported)
    } else {
        match &definition {
            None => Some(Unavailable::DefinitionInvalid),
            Some(found) if !executable(found.program()) => Some(Unavailable::ExecutableMissing),
            Some(_) => None,
        }
    };
    ListedAction {
        action_id: declared.action_id,
        input_schema: declared.input_schema,
        contexts: declared.contexts,
        definition,
        unavailable,
    }
}

impl Catalog {
    /// Reads every `<root>/<plugin_id>/plugin.json`; a missing root has no plugins.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::Io`] for an unreadable root and the manifest errors for any
    /// plugin whose definition is not closed and self-consistent.
    pub fn discover(root: &Path) -> Result<Self, PluginError> {
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    plugins: Vec::new(),
                });
            }
            Err(source) => {
                return Err(PluginError::Io {
                    operation: "listing the plugin root",
                    source,
                });
            }
        };
        let mut directories = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| PluginError::Io {
                operation: "listing the plugin root",
                source,
            })?;
            if entry.path().join(manifest::FILE).is_file() {
                directories.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        directories.sort();
        let plugins = directories
            .into_iter()
            .map(|directory| {
                let declared = manifest::load(root, &directory)?;
                Ok(InstalledPlugin {
                    plugin_id: declared.plugin_id,
                    version: declared.version,
                    enabled: declared.enabled,
                    directory: root.join(&directory),
                    actions: declared
                        .actions
                        .into_iter()
                        .map(|declared_action| action(declared.enabled, declared_action))
                        .collect(),
                })
            })
            .collect::<Result<_, PluginError>>()?;
        Ok(Self { plugins })
    }

    /// The published catalog record.
    #[must_use]
    pub fn listing(&self) -> PluginListResult {
        PluginListResult::new(
            self.plugins
                .iter()
                .map(|plugin| {
                    let actions = plugin
                        .actions
                        .iter()
                        .map(|action| {
                            let listed = PluginAction::new(
                                action.action_id.clone(),
                                action.input_schema.clone(),
                                &action.contexts,
                            );
                            match action.unavailable {
                                Some(reason) => listed.unavailable_because(reason),
                                None => listed,
                            }
                        })
                        .collect();
                    PluginRecord::new(
                        plugin.plugin_id.clone(),
                        plugin.version.clone(),
                        plugin.enabled,
                    )
                    .with_actions(actions)
                })
                .collect(),
        )
    }

    pub(super) fn find(
        &self,
        plugin_id: &PluginId,
        action_id: &PluginActionId,
    ) -> Result<(&InstalledPlugin, &ListedAction), PluginError> {
        self.plugins
            .iter()
            .find(|plugin| &plugin.plugin_id == plugin_id)
            .and_then(|plugin| {
                plugin
                    .actions
                    .iter()
                    .find(|action| &action.action_id == action_id)
                    .map(|action| (plugin, action))
            })
            .ok_or(PluginError::NotFound)
    }
}
