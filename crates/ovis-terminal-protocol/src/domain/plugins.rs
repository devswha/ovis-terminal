//! Typed construction of the `plugin.*` records (plan todo 46).
//!
//! The wire records stay `#[non_exhaustive]`; the plugin service builds them here, so an
//! unavailability reason or a context is always a published token, never free text.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::OpId;
use crate::schema::ids::{PluginActionId, PluginId, PluginRunId, TerminalId};
use crate::schema::methods::plugin::{
    PluginAction, PluginListResult, PluginRecord, PluginRunRecord, PluginRunState,
};
use crate::schema::scalar::{ByteOffset, OpenEnum};

#[cfg(test)]
mod tests;

const RUN_PREFIX: &str = "plugin-run:";

/// Why a listed action cannot be invoked now; the catalog reports it instead of hiding it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unavailable {
    /// The plugin is disabled.
    PluginDisabled,
    /// The action does not declare the operating system the runtime runs on.
    PlatformUnsupported,
    /// The declared program is not an executable file.
    ExecutableMissing,
    /// The argv template or input schema uses a form the runtime does not accept.
    DefinitionInvalid,
}

impl Unavailable {
    /// The published token carried in [`PluginAction::unavailable`].
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PluginDisabled => "plugin_disabled",
            Self::PlatformUnsupported => "platform_unsupported",
            Self::ExecutableMissing => "executable_missing",
            Self::DefinitionInvalid => "definition_invalid",
        }
    }
}

/// Where an action may be invoked; the token listed in [`PluginAction::contexts`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PluginContext {
    /// Invoked for the machine, without a terminal target.
    Machine,
}

impl PluginContext {
    /// The published context token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Machine => "machine",
        }
    }
}

impl PluginRunId {
    /// The run started by operation `op`: one invocation is exactly one run.
    #[must_use]
    pub fn of_operation(op: &OpId) -> Self {
        Self::new(format!("{RUN_PREFIX}{}", op.as_str()))
    }

    /// The invoking operation; `None` for an identity this runtime never minted.
    #[must_use]
    pub fn operation(&self) -> Option<OpId> {
        self.as_str()
            .strip_prefix(RUN_PREFIX)
            .filter(|op| !op.is_empty())
            .map(|op| OpId::new(op.to_owned()))
    }
}

impl PluginAction {
    /// An invocable action with its input schema and the contexts it applies to.
    #[must_use]
    pub fn new(action_id: PluginActionId, input_schema: Value, contexts: &[PluginContext]) -> Self {
        Self {
            action_id,
            input_schema,
            contexts: contexts
                .iter()
                .map(|context| context.as_str().to_owned())
                .collect(),
            unavailable: None,
        }
    }

    /// Marks the action as not invocable for `reason`.
    #[must_use]
    pub fn unavailable_because(mut self, reason: Unavailable) -> Self {
        self.unavailable = Some(reason.as_str().to_owned());
        self
    }
}

impl PluginRecord {
    /// An installed plugin without actions.
    #[must_use]
    pub const fn new(plugin_id: PluginId, version: String, enabled: bool) -> Self {
        Self {
            plugin_id,
            version,
            enabled,
            actions: Vec::new(),
        }
    }

    /// Lists `actions` under the plugin.
    #[must_use]
    pub fn with_actions(mut self, actions: Vec<PluginAction>) -> Self {
        self.actions = actions;
        self
    }
}

impl PluginListResult {
    /// The catalog result.
    #[must_use]
    pub const fn new(plugins: Vec<PluginRecord>) -> Self {
        Self { plugins }
    }
}

impl PluginRunRecord {
    /// A run that has started and retained no output yet.
    #[must_use]
    pub const fn running(run_id: PluginRunId) -> Self {
        Self {
            run_id,
            state: OpenEnum::Known(PluginRunState::Running),
            output_b64: String::new(),
            output_floor: ByteOffset::new(0),
            truncated: false,
            exit_code: None,
            created_terminals: Vec::new(),
        }
    }

    /// Retains the output tail starting at offset `floor`; earlier bytes were truncated.
    #[must_use]
    pub fn with_output(mut self, output_b64: String, floor: ByteOffset) -> Self {
        self.output_b64 = output_b64;
        self.output_floor = floor;
        self.truncated = floor.get() > 0;
        self
    }

    /// Records the settled state and the exit code the process reported, if any.
    #[must_use]
    pub fn settled(mut self, state: PluginRunState, exit_code: Option<i32>) -> Self {
        self.state = OpenEnum::Known(state);
        self.exit_code = exit_code;
        self
    }

    /// Records the terminals the run created.
    #[must_use]
    pub fn with_created(mut self, terminals: Vec<TerminalId>) -> Self {
        self.created_terminals = terminals;
        self
    }
}
