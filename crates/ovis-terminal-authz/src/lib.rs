//! Principals, roles and resource-scoped authority checks for ovis-terminal.
//!
//! A connection is first admitted at its endpoint ([`EndpointPolicy`]), which proves the
//! local account but grants nothing, then authenticates with a runtime-scoped credential
//! ([`Authority::authenticate`]). Every request is authorized against explicit grants of
//! one [`Permission`] over one [`ResourceScope`]; the resulting [`Decision`] is bound to the
//! resolved target and the credential epoch, and the commit or queued write rechecks it
//! with [`Authority::recheck`] so revocation and expiry also stop work already queued.

mod authority;
mod config;
mod credential;
mod endpoint;
mod error;
mod family;
mod identity;
mod permission;
mod rotation;
mod scope;

pub use authority::{
    AccessRequest, AttestRequest, Attestation, Authority, CommitCheck, Decision, IssuedCredential,
    Presentation, Session,
};
pub use config::{AuthorityConfig, AuthorityLimits, OwnerSpec, RuntimeBinding};
pub use credential::{Issuance, IssueRequest, Revocation};
pub use endpoint::{
    Ace, AceKind, Dacl, EndpointPolicy, FileEvidence, LocalAccount, PeerAdmission, PeerEvidence,
    PipeInstance, PipeInstanceEvidence, PipeNamespace, PipeSecurity, RemoteClients,
    RemoteTransport, Transport, UnixSocketEvidence, UnixUid, WindowsSid,
};
pub use error::{AuthzError, EndpointDefect, Exceeded, Limit, MismatchReason};
pub use family::{Identity, IdentityFamily};
pub use identity::{CredentialEpoch, CredentialId, PrincipalId};
pub use permission::{Action, Permission, Role};
pub use rotation::{PrincipalEpoch, Rotated, Rotation, Withdrawn};
pub use scope::{Grant, Placement, ResolvedTarget, ResourceScope, ScopeResource, TargetResource};

/// Cargo package name of this crate, used by the workspace layering record.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Workspace crates this crate builds on; every entry is a declared Cargo dependency.
pub const DEPENDS_ON: &[&str] = &[
    ovis_terminal_protocol::CRATE_NAME,
    ovis_terminal_store::CRATE_NAME,
];
