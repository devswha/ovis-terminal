//! `attach.*`: typed attach outcomes, leases, geometry and stream control as RPCs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::errors::OpId;
use crate::schema::common::{Geometry, Target};
use crate::schema::ids::{ClientId, LeaseId, StreamId};
use crate::schema::limits::StreamMode;
use crate::schema::method::{MethodSpec, method};
use crate::schema::records::{AttachedTerminal, Checkpoint, LeaseRecord, LeaseRole, RetryBoundary};
use crate::schema::scalar::{ByteCount, ByteOffset, EventSeq, LeaseEpoch, OpenEnum, Revision};

/// Explicit takeover request bound to the epoch the caller observed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TakeoverRequest {
    /// Current lease epoch the caller expects to displace.
    pub lease_epoch: LeaseEpoch,
    /// Reason recorded on both owners' events.
    pub reason: String,
}

/// Parameters of `attach.open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AttachOpenParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal to attach to.
    pub target: Target,
    /// Requested role.
    pub role: LeaseRole,
    /// Client label that owns the lease.
    pub client_id: ClientId,
    /// Geometry to apply if this client becomes geometry owner.
    pub geometry: Option<Geometry>,
    /// Stream delivery mode.
    pub stream_mode: StreamMode,
    /// Explicit takeover; never implied.
    pub takeover: Option<TakeoverRequest>,
}

/// Typed outcome of `attach.open`; refusals are records, never terminal text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AttachOutcome {
    /// The stream is live with the requested role.
    Ready,
    /// Another client holds interact; an observe stream is attached.
    Held,
    /// The attach was refused.
    Refused,
    /// The terminal is recovering; retry after the recovery event.
    Recovering,
    /// The terminal has ended.
    Ended,
}

/// Result of `attach.open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AttachOpenResult {
    /// Typed outcome.
    pub outcome: OpenEnum<AttachOutcome>,
    /// Stream identity, never reused.
    pub stream_id: Option<StreamId>,
    /// Framed channel, in channel mode.
    pub channel: Option<u32>,
    /// Lease granted to this client.
    pub lease: Option<LeaseRecord>,
    /// Current interact holder when held.
    pub holder: Option<LeaseRecord>,
    /// Lease displaced by an explicit takeover.
    pub displaced: Option<LeaseRecord>,
    /// Checkpoint that precedes live bytes.
    pub first_paint: Option<Checkpoint>,
    /// Output credit granted at open.
    pub credit_initial: ByteCount,
    /// Terminal and program incarnations the stream is bound to.
    pub terminal: Option<AttachedTerminal>,
    /// What to wait for before retrying a held or recovering attach.
    pub retry: Option<RetryBoundary>,
    /// Stable reason token of a refused or ended outcome.
    pub reason: Option<String>,
    /// Raw output offset of the final byte of an ended terminal.
    pub final_output_offset: Option<ByteOffset>,
}

/// Parameters of `attach.resize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AttachResizeParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal to resize.
    pub target: Target,
    /// Geometry lease.
    pub lease_id: LeaseId,
    /// Requested size.
    pub geometry: Geometry,
}

/// Outcome of a resize request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ResizeOutcome {
    /// The new grid was applied.
    Applied,
    /// The grid already had this size.
    Unchanged,
    /// A later resize superseded this one.
    Superseded,
}

/// Result of `attach.resize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AttachResizeResult {
    /// Outcome.
    pub outcome: OpenEnum<ResizeOutcome>,
    /// Committed geometry.
    pub geometry: Geometry,
    /// Screen revision of the commit.
    pub revision: Revision,
}

/// Parameters of methods that act with a lease on a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AttachLeaseParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal.
    pub target: Target,
    /// Caller's lease.
    pub lease_id: LeaseId,
}

/// Result of `attach.release_geometry` and `attach.focus`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AttachLeaseResult {
    /// Geometry owner after the change.
    pub geometry_owner: Option<LeaseRecord>,
    /// Sequence of the lease event.
    pub seq: EventSeq,
}

/// Parameters of methods that act on one stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AttachStreamParams {
    /// Stream.
    pub stream_id: StreamId,
}

/// Parameters of `attach.credit` for plain-mode streams.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AttachCreditParams {
    /// Stream.
    pub stream_id: StreamId,
    /// Cumulative filtered-stream offset the client has consumed.
    pub acknowledged: ByteOffset,
}

/// Result of stream-control methods.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AttachStreamResult {
    /// Stream.
    pub stream_id: StreamId,
    /// Filtered stream offset after the change.
    pub stream_offset: ByteOffset,
    /// Checkpoint, for `attach.repaint`.
    pub checkpoint: Option<Checkpoint>,
}

/// Parameters of `attach.takeover`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AttachTakeoverParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal.
    pub target: Target,
    /// Caller's stream, which receives the interact lease.
    pub stream_id: StreamId,
    /// Takeover request.
    pub takeover: TakeoverRequest,
}

/// Result of `attach.takeover`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AttachTakeoverResult {
    /// Lease acquired.
    pub acquired: LeaseRecord,
    /// Lease displaced.
    pub displaced: LeaseRecord,
    /// Sequence of the takeover event.
    pub seq: EventSeq,
}

/// Parameters of `attach.leases`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AttachLeasesParams {
    /// Terminal.
    pub target: Target,
}

/// Result of `attach.leases`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AttachLeasesResult {
    /// Interact lease, when held.
    pub interact: Option<LeaseRecord>,
    /// Geometry owner, when any.
    pub geometry_owner: Option<LeaseRecord>,
    /// Observe leases.
    pub observers: Vec<LeaseRecord>,
}

/// Methods of this namespace.
pub(crate) const METHODS: &[MethodSpec] = &[
    method! {"attach.open", Mutation, Terminal, AttachOpenParams => AttachOpenResult,
    "Attach with a typed outcome and an offset-fenced first paint.", [NeedsFramed, LeaseConflict, LeaseEpochMismatch, Finished, Recovering, StreamLimit, InvalidGeometry, TakeoverNotAuthorized]},
    method! {"attach.resize", Mutation, Terminal, AttachResizeParams => AttachResizeResult,
    "Resize under the geometry lease.", [NoLease, NotGeometryOwner, InvalidGeometry, ObserveForbiddenOp]},
    method! {"attach.release_geometry", Mutation, Terminal, AttachLeaseParams => AttachLeaseResult,
    "Release geometry ownership to the next eligible interact client.", [NoLease]},
    method! {"attach.focus", Mutation, Terminal, AttachLeaseParams => AttachLeaseResult,
    "Record client focus; never grants input.", [NoLease]},
    method! {"attach.repaint", Control, Lookup, AttachStreamParams => AttachStreamResult,
    "Send a fresh checkpoint on a stream.", [StreamNotFound]},
    method! {"attach.credit", Control, Lookup, AttachCreditParams => AttachStreamResult,
    "Acknowledge consumed bytes on a plain-mode stream.", [StreamNotFound, InvalidCredit]},
    method! {"attach.close", Control, Lookup, AttachStreamParams => AttachStreamResult,
    "Close a stream and release its lease.", [StreamNotFound]},
    method! {"attach.takeover", Mutation, Terminal, AttachTakeoverParams => AttachTakeoverResult,
    "Take over the interact lease with explicit authority and epoch.", [LeaseEpochMismatch, TakeoverNotAuthorized, NoLeaseToTake, SelfLease]},
    method! {"attach.leases", Query, Terminal, AttachLeasesParams => AttachLeasesResult,
    "List leases on a terminal.", []},
];
