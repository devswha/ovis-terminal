//! `machine.*`: stable remote machines over byte-transparent transports.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::errors::OpId;
use crate::schema::common::MachineRef;
use crate::schema::ids::{BuildVersion, MachineId, MachineName, RuntimeLifetimeId};
use crate::schema::method::{MethodSpec, method};
use crate::schema::scalar::{ByteCount, ByteOffset, DurationMs, EventSeq, OpenEnum, Revision};

/// Connection state of a machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MachineState {
    /// Connected and authenticated.
    Connected,
    /// Connecting.
    Connecting,
    /// Unreachable; resources keep their last-known revision.
    Unreachable,
}

/// One machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct MachineRecord {
    /// Stable identity.
    pub machine_id: MachineId,
    /// Current name.
    pub name: MachineName,
    /// Former names still resolvable during the grace period.
    pub aliases: Vec<MachineName>,
    /// Connection state.
    pub state: OpenEnum<MachineState>,
    /// Remote runtime lifetime, when known.
    pub remote_lifetime: Option<RuntimeLifetimeId>,
    /// Last remote sequence mirrored locally.
    pub last_seen_seq: Option<EventSeq>,
    /// Remote runtime version, when known.
    pub version: Option<BuildVersion>,
    /// Record revision.
    pub revision: Revision,
}

/// How the local runtime reaches a machine; arguments are never shell-interpolated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum MachineTransport {
    /// SSH to a host alias or `user@host`.
    Ssh(String),
    /// Custom argument vector that carries stdin and stdout unchanged.
    Argv(Vec<String>),
}

/// Parameters of `machine.add`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MachineAddParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Machine name.
    pub name: MachineName,
    /// Transport.
    pub transport: MachineTransport,
    /// Credential that authorizes the remote runtime.
    pub credential: String,
}

/// Parameters of machine reads and removals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MachineRefParams {
    /// Machine.
    pub machine: MachineRef,
}

/// Parameters of machine mutations that only name a machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MachineMutateParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Machine.
    pub machine: MachineRef,
}

/// Parameters of `machine.rename`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MachineRenameParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Machine.
    pub machine: MachineRef,
    /// New name.
    pub name: MachineName,
}

/// Result of `machine.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct MachineListResult {
    /// Machines.
    pub machines: Vec<MachineRecord>,
}

/// Parameters of `machine.exec`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MachineExecParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Machine; absent runs locally.
    pub machine: Option<MachineRef>,
    /// Argument vector.
    pub argv: Vec<String>,
    /// Working directory.
    pub cwd: Option<String>,
    /// Environment additions.
    pub env: Option<BTreeMap<String, String>>,
    /// Standard input, base64-encoded.
    pub stdin_b64: Option<String>,
    /// Run bound.
    pub timeout_ms: DurationMs,
}

/// Result of `machine.exec`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct MachineExecResult {
    /// Exit code, when the program exited.
    pub exit_code: Option<i32>,
    /// Standard output, base64-encoded and bounded.
    pub stdout_b64: String,
    /// Standard error, base64-encoded and bounded.
    pub stderr_b64: String,
    /// Whether output was truncated.
    pub truncated: bool,
    /// Run time.
    pub duration_ms: DurationMs,
}

/// File operation of `machine.file`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum FileOperation {
    /// Read a byte range.
    Read {
        /// First byte.
        offset: ByteOffset,
        /// Maximum bytes.
        limit: ByteCount,
    },
    /// Replace the file with base64 content.
    Write(String),
}

/// Parameters of `machine.file`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MachineFileParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Machine; absent acts locally.
    pub machine: Option<MachineRef>,
    /// Path inside the authorized root.
    pub path: String,
    /// Operation.
    pub operation: FileOperation,
}

/// Result of `machine.file`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct MachineFileResult {
    /// Content read, base64-encoded.
    pub content_b64: Option<String>,
    /// Bytes read or written.
    pub bytes: ByteCount,
    /// Whether a read stopped at its limit.
    pub truncated: bool,
}

/// Methods of this namespace.
pub(crate) const METHODS: &[MethodSpec] = &[
    method! {"machine.add", Mutation, Untargeted, MachineAddParams => MachineRecord,
    "Add a machine with explicit transport and trust.", [DuplicateName, InvalidName, TransportFailed, AuthFailed, HostKeyUnknown, RemoteIncompatible, InstallRefused]},
    method! {"machine.list", Query, Untargeted, crate::schema::common::NoParams => MachineListResult,
    "List machines.", []},
    method! {"machine.get", Query, Lookup, MachineRefParams => MachineRecord,
    "Read one machine.", []},
    method! {"machine.rename", Mutation, Lookup, MachineRenameParams => MachineRecord,
    "Rename a machine, keeping the old name as an alias.", [DuplicateName, InvalidName]},
    method! {"machine.rm", Mutation, Lookup, MachineMutateParams => MachineRecord,
    "Remove a machine and tombstone its terminals.", []},
    method! {"machine.login", Mutation, Lookup, MachineMutateParams => MachineRecord,
    "Authenticate to a machine interactively.", [NeedsTty, AuthFailed]},
    method! {"machine.exec", Mutation, Untargeted, MachineExecParams => MachineExecResult,
    "Run an argument vector with bounded output.", [MachineUnreachable, SpawnFailed, OutputTruncated]},
    method! {"machine.file", Mutation, Untargeted, MachineFileParams => MachineFileResult,
    "Read or write a file inside the authorized root.", [MachineUnreachable, PathNotFound, PathForbidden, TooLarge]},
];
