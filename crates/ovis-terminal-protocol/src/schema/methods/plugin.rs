//! `plugin.*`: scoped plugin catalogs and durable, bounded run outcomes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::OpId;
use crate::schema::common::{NoParams, Target};
use crate::schema::ids::{PluginActionId, PluginId, PluginRunId, TerminalId};
use crate::schema::method::{MethodSpec, method};
use crate::schema::scalar::{ByteOffset, OpenEnum};

/// One plugin action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct PluginAction {
    /// Action identity.
    pub action_id: PluginActionId,
    /// JSON Schema of the action's input.
    pub input_schema: Value,
    /// Contexts the action applies to.
    pub contexts: Vec<String>,
    /// Why the action cannot be invoked now, when it cannot.
    pub unavailable: Option<String>,
}

/// One installed plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct PluginRecord {
    /// Plugin identity.
    pub plugin_id: PluginId,
    /// Plugin version.
    pub version: String,
    /// Whether the plugin is enabled.
    pub enabled: bool,
    /// Actions.
    pub actions: Vec<PluginAction>,
}

/// Result of `plugin.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct PluginListResult {
    /// Plugins.
    pub plugins: Vec<PluginRecord>,
}

/// Parameters of `plugin.invoke`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PluginInvokeParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Plugin.
    pub plugin_id: PluginId,
    /// Action.
    pub action_id: PluginActionId,
    /// Action input, validated against the action's schema.
    pub input: Value,
    /// Terminal context, when the action applies to one.
    pub target: Option<Target>,
}

/// State of a plugin run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PluginRunState {
    /// Running.
    Running,
    /// Exited with code zero.
    Succeeded,
    /// Exited with a failure.
    Failed,
    /// Cancelled.
    Cancelled,
}

/// Durable record of one plugin run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct PluginRunRecord {
    /// Run identity.
    pub run_id: PluginRunId,
    /// State.
    pub state: OpenEnum<PluginRunState>,
    /// Bounded output, base64-encoded.
    pub output_b64: String,
    /// Offset of the oldest retained output byte.
    pub output_floor: ByteOffset,
    /// Whether output was truncated.
    pub truncated: bool,
    /// Exit code, once exited.
    pub exit_code: Option<i32>,
    /// Terminals the run created.
    pub created_terminals: Vec<TerminalId>,
}

/// Parameters of `plugin.run_get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PluginRunGetParams {
    /// Run.
    pub run_id: PluginRunId,
}

/// Methods of this namespace.
pub(crate) const METHODS: &[MethodSpec] = &[
    method! {"plugin.list", Query, Untargeted, NoParams => PluginListResult,
    "List plugins with action schemas and unavailability reasons.", []},
    method! {"plugin.invoke", Mutation, Lookup, PluginInvokeParams => PluginRunRecord,
    "Invoke a plugin action with explicit grants; returns a durable run.", [PluginDisabled, ActionUnavailable, SpawnFailed, AmbiguousTarget, StaleTarget]},
    method! {"plugin.run_get", Query, Lookup, PluginRunGetParams => PluginRunRecord,
    "Read a plugin run's state, bounded output and created resources.", []},
];
