//! Opaque identity newtypes; each identity domain is a distinct schema type.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::errors::{FieldPath, OpId, TargetId};
use crate::rpc::RequestId;

fn opaque_string_schema(description: &str) -> Schema {
    json_schema!({ "type": "string", "description": description })
}

/// Implements the schema of an existing string newtype.
macro_rules! string_schema {
    ($name:ident, $description:literal) => {
        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                Cow::Borrowed(stringify!($name))
            }

            fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
                opaque_string_schema($description)
            }
        }
    };
}

string_schema!(
    OpId,
    "Client-minted operation identity used for idempotent retries."
);
string_schema!(
    TargetId,
    "Stable identity of the resource an error refers to."
);
string_schema!(FieldPath, "Dotted path to the offending input field.");
string_schema!(RequestId, "JSON-RPC request identity; always a string.");

/// Declares an opaque string identity with its own schema name.
macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Wraps an identity string.
            #[must_use]
            pub const fn new(value: String) -> Self {
                Self(value)
            }

            /// Borrows the identity string.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        string_schema!($name, "Opaque identity string.");
    };
}

string_id!(
    /// Stable terminal identity, never reused inside a runtime lifetime.
    TerminalId
);
string_id!(
    /// Terminal address such as `t7`, `name`, `machine/t7` or `machine/name`.
    TerminalAddress
);
string_id!(
    /// Human-chosen terminal name, unique among live terminals of a machine.
    TerminalName
);
string_id!(
    /// Stable machine identity of one runtime install.
    MachineId
);
string_id!(
    /// Human-chosen machine name or alias.
    MachineName
);
string_id!(
    /// Identity of one holder-supervisor lifetime; survives control restarts.
    RuntimeLifetimeId
);
string_id!(
    /// Identity of one control-server process; changes on every restart.
    ServerBootId
);
string_id!(
    /// Identity of one attach output stream.
    StreamId
);
string_id!(
    /// Identity of one interact, observe or geometry lease.
    LeaseId
);
string_id!(
    /// Identity of one event subscription.
    SubscriptionId
);
string_id!(
    /// Identity of one runtime-scoped credential, `c<n>`.
    CredentialId
);
string_id!(
    /// Name of the principal a credential authenticates.
    PrincipalName
);
string_id!(
    /// Identity of one prompt record.
    PromptId
);
string_id!(
    /// Identity of one registered agent binding.
    AgentId
);
string_id!(
    /// Identity of one parent-to-child note.
    NoteId
);
string_id!(
    /// Identity of one conversation history.
    HistoryId
);
string_id!(
    /// Identity of one record inside a conversation history.
    RecordId
);
string_id!(
    /// Capability token handed to a spawned program for reports it owns.
    ProcessToken
);
string_id!(
    /// Opaque start token proving which program a process identifier names.
    StartToken
);
string_id!(
    /// Client label that scopes selections and lease ownership.
    ClientId
);
string_id!(
    /// Stable workspace identity.
    WorkspaceId
);
string_id!(
    /// Stable tab identity.
    TabId
);
string_id!(
    /// Stable pane identity.
    PaneId
);
string_id!(
    /// Stable split-node identity, independent of its position in the tree.
    SplitNodeId
);
string_id!(
    /// Stable repository identity.
    RepositoryId
);
string_id!(
    /// Stable checkout (worktree) identity.
    CheckoutId
);
string_id!(
    /// Stable plugin identity.
    PluginId
);
string_id!(
    /// Identity of one plugin action.
    PluginActionId
);
string_id!(
    /// Durable identity of one plugin run.
    PluginRunId
);
string_id!(
    /// Identity of one staged update.
    UpdateId
);
string_id!(
    /// Stable template name.
    TemplateName
);
string_id!(
    /// Document name.
    DocName
);
string_id!(
    /// JSON-RPC method name, such as `terminal.get`.
    MethodName
);
string_id!(
    /// Stable machine-readable error kind token.
    ErrorKindToken
);
string_id!(
    /// Content digest such as `sha256:<hex>`.
    Digest
);
string_id!(
    /// Release version string of a runtime build.
    BuildVersion
);
string_id!(
    /// Identity of an installed service registration.
    ServiceLabel
);

/// Operating-system process identifier; fits 32 bits on every supported platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ProcessPid(u32);

impl ProcessPid {
    /// Wraps a native process identifier.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the native process identifier.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}
