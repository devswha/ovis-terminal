//! Failures of plugin discovery, invocation and run lookup.

use ovis_terminal_protocol::domain::plugins::Unavailable;

/// A plugin refusal or failure; every variant has a stable kind and a recovery hint.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PluginError {
    /// The plugin directory could not be read.
    #[error("the plugin directory could not be read while {operation}")]
    Io {
        /// What the service was doing.
        operation: &'static str,
        /// The filesystem failure.
        #[source]
        source: std::io::Error,
    },
    /// A manifest is not a closed, well-formed plugin definition.
    #[error("the manifest of plugin directory {directory} is invalid")]
    ManifestInvalid {
        /// The plugin directory name.
        directory: String,
        /// Why the manifest was rejected.
        #[source]
        source: serde_json::Error,
    },
    /// A manifest's identity disagrees with its directory or repeats an action.
    #[error("the manifest of plugin directory {directory} declares an inconsistent identity")]
    IdentityMismatch {
        /// The plugin directory name.
        directory: String,
    },
    /// No installed plugin has this identity and action.
    #[error("no installed plugin action has this identity")]
    NotFound,
    /// The plugin is installed but disabled.
    #[error("the plugin is disabled")]
    Disabled,
    /// The action is listed but cannot be invoked.
    #[error("the action is not invocable: {}", reason.as_str())]
    Unavailable {
        /// The reason the catalog lists for the action.
        reason: Unavailable,
    },
    /// The action does not apply to the requested context.
    #[error("the action does not apply to a terminal target")]
    ContextUnsupported,
    /// The input does not match the action's schema.
    #[error("the action input does not match its schema at {field}")]
    InvalidInput {
        /// Dotted path of the offending input field.
        field: String,
    },
    /// The declared program could not be started; no output was produced.
    #[error("the declared program could not be started")]
    SpawnFailed(#[source] std::io::Error),
    /// Too many runs are retained; a settled run must age out first.
    #[error("the plugin run table is full")]
    RunCapacity,
    /// A run with this identity is already retained.
    #[error("a run with this identity already exists")]
    RunExists,
    /// A shared run table was poisoned by a panicking holder.
    #[error("the plugin run table is unusable")]
    Poisoned,
}

impl PluginError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Io { .. } | Self::Poisoned => "internal",
            Self::ManifestInvalid { .. } | Self::IdentityMismatch { .. } => "plugin_invalid",
            Self::NotFound => "not_found",
            Self::Disabled => "plugin_disabled",
            Self::Unavailable { .. } | Self::ContextUnsupported => "action_unavailable",
            Self::InvalidInput { .. } => "invalid_params",
            Self::SpawnFailed(_) => "spawn_failed",
            Self::RunCapacity => "limit_exceeded",
            Self::RunExists => "op_reused",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Io { .. } => "make the plugin directory readable by the runtime",
            Self::ManifestInvalid { .. } | Self::IdentityMismatch { .. } => {
                "fix plugin.json so it matches its directory and the closed manifest format"
            }
            Self::NotFound => "list plugins and invoke a listed plugin action",
            Self::Disabled => "enable the plugin before invoking its actions",
            Self::Unavailable { .. } => "read the action's unavailable reason in plugin.list",
            Self::ContextUnsupported => "invoke the action without a terminal target",
            Self::InvalidInput { .. } => "send input matching the action's input_schema",
            Self::SpawnFailed(_) => "check the declared program and its permissions",
            Self::RunCapacity => "wait for running plugin actions to settle, then retry",
            Self::RunExists => "inspect the original run instead of reusing its operation",
            Self::Poisoned => "restart the runtime",
        }
    }
}

#[cfg(test)]
mod tests {
    use ovis_terminal_protocol::domain::plugins::Unavailable;

    use super::PluginError;

    #[test]
    fn display_never_repeats_the_kind_token() {
        let errors = [
            PluginError::NotFound,
            PluginError::Disabled,
            PluginError::Unavailable {
                reason: Unavailable::ExecutableMissing,
            },
            PluginError::ContextUnsupported,
            PluginError::InvalidInput {
                field: "input.name".to_owned(),
            },
            PluginError::RunCapacity,
        ];
        for error in errors {
            assert!(!error.to_string().contains(error.kind()), "{error}");
            assert_ne!(error.hint(), "");
        }
    }
}
