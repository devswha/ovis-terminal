//! `rpc.discover` and `schema.*`: the published contract, served without side effects.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::schema::common::NoParams;
use crate::schema::ids::{ErrorKindToken, MethodName};
use crate::schema::limits::{Capabilities, Limits};
use crate::schema::method::{MethodSpec, method};
use crate::schema::scalar::ProtocolGeneration;

/// Result of `rpc.discover`: the OpenRPC document of this build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct RpcDiscoverResult(Value);

/// One published error kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ErrorKindEntry {
    /// Stable token.
    pub kind: ErrorKindToken,
    /// JSON-RPC code the kind travels with.
    pub code: i32,
    /// What the kind means.
    pub description: String,
}

/// Result of `schema.errors`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct SchemaErrorsResult {
    /// Every error kind.
    pub kinds: Vec<ErrorKindEntry>,
}

/// One published event type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct EventTypeEntry {
    /// Event type.
    pub name: MethodName,
    /// JSON Schema of the event data.
    pub schema: Value,
}

/// Result of `schema.events`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct SchemaEventsResult {
    /// Every event type.
    pub events: Vec<EventTypeEntry>,
}

/// Result of `schema.capabilities`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct SchemaCapabilitiesResult {
    /// Protocol generation.
    pub generation: ProtocolGeneration,
    /// Served capabilities.
    pub capabilities: Capabilities,
    /// Advertised limits.
    pub limits: Limits,
}

/// Methods of this namespace.
pub(crate) const METHODS: &[MethodSpec] = &[
    method! {"rpc.discover", Query, Untargeted, NoParams => RpcDiscoverResult,
    "Return the OpenRPC document of this build.", []},
    method! {"schema.errors", Query, Untargeted, NoParams => SchemaErrorsResult,
    "List every error kind with its code and meaning.", []},
    method! {"schema.events", Query, Untargeted, NoParams => SchemaEventsResult,
    "List every event type with its data schema.", []},
    method! {"schema.capabilities", Query, Untargeted, NoParams => SchemaCapabilitiesResult,
    "Return generation, capabilities and limits.", []},
];
