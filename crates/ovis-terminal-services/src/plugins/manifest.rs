//! The closed `plugin.json` format read from one plugin directory.

use std::collections::BTreeSet;
use std::io::Read as _;
use std::path::Path;

use ovis_terminal_protocol::domain::plugins::PluginContext;
use ovis_terminal_protocol::schema::ids::{PluginActionId, PluginId};
use serde::Deserialize;
use serde_json::{Value, json};

use super::PluginError;

pub(super) const FILE: &str = "plugin.json";
const MAX_BYTES: u64 = 64 << 10;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub(super) plugin_id: PluginId,
    pub(super) version: String,
    pub(super) enabled: bool,
    pub(super) actions: Vec<ActionManifest>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ActionManifest {
    pub(super) action_id: PluginActionId,
    /// Absolute program path followed by literal or `{field}` arguments; never a shell line.
    pub(super) argv: Vec<String>,
    #[serde(default = "empty_input")]
    pub(super) input_schema: Value,
    pub(super) contexts: Vec<PluginContext>,
    /// Operating systems the action runs on; absent means every platform.
    #[serde(default)]
    pub(super) platforms: Option<Vec<Platform>>,
}

fn empty_input() -> Value {
    json!({"type": "object", "properties": {}, "additionalProperties": false})
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Platform {
    Linux,
    Macos,
    Windows,
}

impl Platform {
    pub(super) const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

impl ActionManifest {
    pub(super) fn runs_here(&self) -> bool {
        self.platforms
            .as_ref()
            .is_none_or(|platforms| platforms.contains(&Platform::current()))
    }
}

pub(super) fn load(root: &Path, directory: &str) -> Result<Manifest, PluginError> {
    let invalid = || PluginError::IdentityMismatch {
        directory: directory.to_owned(),
    };
    let file =
        std::fs::File::open(root.join(directory).join(FILE)).map_err(|source| PluginError::Io {
            operation: "opening a plugin manifest",
            source,
        })?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| PluginError::Io {
            operation: "reading a plugin manifest",
            source,
        })?;
    if u64::try_from(bytes.len()).map_or(true, |len| len > MAX_BYTES) {
        return Err(invalid());
    }
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|source| PluginError::ManifestInvalid {
            directory: directory.to_owned(),
            source,
        })?;
    let unique: BTreeSet<&PluginActionId> = manifest
        .actions
        .iter()
        .map(|action| &action.action_id)
        .collect();
    if manifest.plugin_id.as_str() != directory || unique.len() != manifest.actions.len() {
        return Err(invalid());
    }
    Ok(manifest)
}
