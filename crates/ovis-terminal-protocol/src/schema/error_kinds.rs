//! Domain error kinds every v1 method may return, beyond the envelope kinds.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Stable domain error kind published in the schema; clients branch on its token.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DomainErrorKind {
    /// The connection has not completed hello or presented no credential.
    Unauthenticated,
    /// The principal lacks the scope the method requires.
    Forbidden,
    /// The request deadline passed before the method settled.
    DeadlineExceeded,
    /// The caller cancelled the request before commit.
    Cancelled,
    /// A bounded queue is full; retry after the advertised delay.
    Backpressure,
    /// The operation id is older than the retention window.
    OpExpired,
    /// The operation id was used with different parameters.
    OpReused,
    /// The durable commit failed; no effect is visible.
    PersistenceFailed,
    /// Effects may have begun but their outcome is not provable.
    OutcomeUnknown,
    /// A name resolves to several terminals.
    AmbiguousTarget,
    /// The target no longer has the expected incarnation.
    StaleTarget,
    /// The owning machine cannot be reached.
    MachineUnreachable,
    /// The requested protocol generation is not served.
    UnsupportedGeneration,
    /// The presented credential is not valid.
    InvalidCredential,
    /// The presented credential has expired.
    CredentialExpired,
    /// The endpoint descriptor names a runtime that is gone.
    StaleEndpoint,
    /// The control server is already running.
    AlreadyRunning,
    /// The control server is already stopped.
    AlreadyStopped,
    /// The program could not be spawned.
    SpawnFailed,
    /// Another live resource already has this name.
    DuplicateName,
    /// The name violates the naming rules.
    InvalidName,
    /// No template has this name and version.
    TemplateNotFound,
    /// The template definition is invalid.
    TemplateInvalid,
    /// The named parent does not exist.
    ParentNotFound,
    /// The named parent incarnation has finished.
    ParentFinished,
    /// The geometry is outside the accepted bounds.
    InvalidGeometry,
    /// A configured count or size limit would be exceeded.
    LimitExceeded,
    /// The program has already finished.
    AlreadyFinished,
    /// The terminal still runs a program.
    StillLive,
    /// The terminal still has live children.
    HasChildren,
    /// The terminal is not in the lost state.
    NotLost,
    /// Recovery is disabled for this terminal.
    RecoveryDisabled,
    /// The platform cannot report the requested field.
    UnsupportedField,
    /// An interact lease exists and the caller does not hold it.
    NoLease,
    /// The caller's lease was revoked.
    LeaseRevoked,
    /// Another client holds the requested lease.
    LeaseConflict,
    /// The lease epoch does not match the current one.
    LeaseEpochMismatch,
    /// A geometry request names an epoch a newer transition superseded.
    StaleGeometry,
    /// The program is blocked on a prompt; answer it instead.
    Blocked,
    /// The program has not declared readiness.
    NotReady,
    /// The terminal is recovering.
    Recovering,
    /// The terminal has finished.
    Finished,
    /// The input queue is full.
    InputOverflow,
    /// The input cannot be encoded for the current terminal modes.
    InvalidEncoding,
    /// The program was replaced since admission.
    ProcessReplaced,
    /// Multiline text cannot be pasted safely in the current modes.
    PasteRefused,
    /// The terminal modes changed between admission and write.
    ModeMismatch,
    /// The key name is not known.
    UnknownKey,
    /// Raw input requires its own explicit grant.
    RawNotAuthorized,
    /// The requested text format is not supported.
    FormatUnsupported,
    /// The reply would exceed the maximum frame size.
    FrameTooLarge,
    /// The cursor points before the retained floor.
    CursorExpired,
    /// History is unavailable for the stated reason.
    HistoryUnavailable,
    /// The revision no longer matches the current one.
    RevisionMismatch,
    /// The pattern is not valid.
    InvalidPattern,
    /// The output offset was evicted from retention.
    OffsetEvicted,
    /// Channel streams require a framed connection.
    NeedsFramed,
    /// An observe lease cannot perform this operation.
    ObserveForbiddenOp,
    /// The stream limit is reached.
    StreamLimit,
    /// The principal may not take over this lease.
    TakeoverNotAuthorized,
    /// Only the geometry owner may resize.
    NotGeometryOwner,
    /// The stream does not exist or has closed.
    StreamNotFound,
    /// The credit acknowledges bytes never sent.
    InvalidCredit,
    /// There is no lease to take over.
    NoLeaseToTake,
    /// The caller already holds this lease.
    SelfLease,
    /// The event cursor belongs to another runtime lifetime.
    LifetimeMismatch,
    /// The event filter is invalid.
    FilterInvalid,
    /// The subscription limit is reached.
    SubscriptionLimit,
    /// The acknowledged sequence is ahead of delivery.
    SeqAhead,
    /// The report sequence is not newer than the current one.
    StaleSeq,
    /// The caller does not own the reporting process or prompt.
    NotOwner,
    /// The status record is invalid.
    InvalidRecord,
    /// The prompt version changed.
    VersionConflict,
    /// The prompt is closed.
    PromptClosed,
    /// The answer does not match the prompt schema.
    AnswerInvalid,
    /// The prompt cannot deliver this kind of answer.
    DeliveryUnsupported,
    /// The principal may not answer this prompt.
    NotAuthorized,
    /// The prompt definition is invalid.
    InvalidPrompt,
    /// The record is already closed.
    AlreadyClosed,
    /// The turn is not known.
    UnknownTurn,
    /// The agent registration is invalid.
    InvalidAgent,
    /// Another agent is bound to this program.
    AgentConflict,
    /// The session binding is invalid.
    InvalidSession,
    /// Notes are disabled for this parent.
    NotesDisabled,
    /// The terminal is not a child of the parent.
    NotChild,
    /// Typed delivery needs the parent's interact lease.
    NoLeaseForTyped,
    /// The conversation history changed under the cursor.
    HistoryChanged,
    /// The conversation is unavailable for the stated reason.
    ConversationUnavailable,
    /// The provider store is unavailable.
    StoreUnavailable,
    /// The document version does not match.
    Conflict,
    /// The payload exceeds its size limit.
    TooLarge,
    /// The transport to the machine failed.
    TransportFailed,
    /// Authentication with the machine failed.
    AuthFailed,
    /// The host key is not trusted.
    HostKeyUnknown,
    /// The remote runtime is incompatible.
    RemoteIncompatible,
    /// The remote install was refused.
    InstallRefused,
    /// No expected machine identity was configured for the remote runtime.
    TrustRequired,
    /// The remote runtime proved a machine identity other than the expected one.
    TrustMismatch,
    /// The boot proof does not answer this connection's fresh challenge.
    BootProofStale,
    /// The remote runtime did not complete the handshake within its bound.
    HandshakeTimeout,
    /// The machine is already added under another name.
    MachineExists,
    /// The operation needs an interactive terminal.
    NeedsTty,
    /// An integrity check failed.
    IntegrityFailed,
    /// The build is incompatible with this runtime.
    Incompatible,
    /// The output exceeded its bound.
    OutputTruncated,
    /// The path does not exist.
    PathNotFound,
    /// The path is outside the authorized scope.
    PathForbidden,
    /// The edit matches more than one location.
    EditAmbiguous,
    /// The operation would affect resources beyond the named target.
    ScopeRequired,
    /// The checkout has uncommitted changes.
    CheckoutDirty,
    /// The checkout is locked.
    CheckoutLocked,
    /// A version-control command failed.
    VcsFailed,
    /// The plugin is disabled.
    PluginDisabled,
    /// The plugin action is not invocable in this context.
    ActionUnavailable,
    /// No update is staged.
    NoStagedUpdate,
    /// Rolling back the update failed.
    RollbackFailed,
    /// The platform has no supported service manager.
    ServiceUnsupported,
    /// A service registration with this label already exists.
    ServiceExists,
    /// The credential was revoked after the session or decision was made.
    CredentialRevoked,
    /// An authorization decision was presented for another action, target or authority.
    DecisionMismatch,
    /// An issuance asks for more authority than the issuer holds.
    GrantExceedsIssuer,
    /// The serving endpoint is reachable by accounts other than its owner.
    EndpointInsecure,
    /// The connecting peer is not the account that owns the endpoint.
    PeerRejected,
}
