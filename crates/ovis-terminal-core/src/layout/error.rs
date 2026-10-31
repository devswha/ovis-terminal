//! Typed layout boundary failures; none is turned into a clamp or a default.

use ovis_terminal_protocol::errors::{BoundaryError, ErrorData, ErrorKind, TargetId};
use ovis_terminal_protocol::schema::scalar::Revision;
use serde::{Deserialize, Serialize};

use crate::events::ResourceId;

/// Why a ratio or resize request cannot be applied exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum GeometryReason {
    /// The resulting first-child share lies outside the supported permille range.
    OutOfRange,
    /// The resize amount does not land on a whole permille of the split.
    Unrepresentable,
}

/// The rejected value and the exact accepted range; nothing was clamped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct GeometryRejection {
    /// Why the request was rejected.
    pub reason: GeometryReason,
    /// Requested value, in the request's unit (permille or tab ppm).
    pub requested: i64,
    /// Smallest accepted value in that unit.
    pub min: i64,
    /// Largest accepted value in that unit.
    pub max: i64,
    /// Accepted values are multiples of this step above `min`.
    pub step: i64,
}

/// A rejected layout request; no layout state changed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LayoutError {
    /// Authority denied the request at the commit boundary.
    #[error(transparent)]
    Authority(#[from] ovis_terminal_authz::AuthzError),
    /// Operation admission, deduplication or completion failed.
    #[error(transparent)]
    Operation(#[from] crate::ops::OperationError),
    /// Durable event or snapshot coordination failed.
    #[error(transparent)]
    Event(#[from] crate::events::EventError),
    /// A new identity could not be reserved.
    #[error(transparent)]
    Reservation(#[from] crate::registry::ReservationError),
    /// A layout record could not be encoded.
    #[error("layout records could not be encoded or decoded")]
    Serialization(#[from] serde_json::Error),
    /// The request does not match its closed schema.
    #[error("the layout request does not match its closed schema")]
    InvalidJson(#[source] serde_json::Error),
    /// A field is outside its accepted domain.
    #[error("the layout request field `{field}` is invalid")]
    InvalidParams {
        /// Dotted path of the rejected field.
        field: &'static str,
    },
    /// No resource has the named identity.
    #[error("no layout resource matches the requested identity")]
    NotFound {
        /// The identity that was named.
        resource: ResourceId,
    },
    /// A ratio or resize cannot be applied without clamping or rounding.
    #[error("the requested geometry cannot be applied exactly")]
    InvalidGeometry(GeometryRejection),
    /// The request was computed against an older layout.
    #[error("the layout changed after the expected revision")]
    RevisionMismatch {
        /// Revision the caller computed against.
        expected: Revision,
        /// Current layout revision.
        current: Revision,
    },
    /// Closing the target would also close linked workspaces.
    #[error("closing the target would close linked workspaces")]
    GroupScopeRequired {
        /// Every resource a group close would remove.
        affected: Vec<ResourceId>,
    },
    /// A bounded layout collection is full.
    #[error("the layout would exceed its configured size")]
    LimitExceeded,
    /// A launch was requested without a launcher bound to the call.
    #[error("no launcher is bound to this layout call")]
    LauncherUnavailable,
}

impl LayoutError {
    /// Stable machine token.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Authority(error) => error.kind(),
            Self::Operation(error) => error.kind(),
            Self::Event(error) => error.kind(),
            Self::Reservation(error) => error.kind(),
            Self::Serialization(_) => "persistence_failed",
            Self::InvalidJson(_) | Self::InvalidParams { .. } => "invalid_params",
            Self::NotFound { .. } => "not_found",
            Self::InvalidGeometry(_) => "invalid_geometry",
            Self::RevisionMismatch { .. } => "revision_mismatch",
            Self::GroupScopeRequired { .. } => "group_scope_required",
            Self::LimitExceeded => "limit_exceeded",
            Self::LauncherUnavailable => "launcher_unavailable",
        }
    }

    /// Recovery guidance, independent of Display prose.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Authority(error) => error.hint(),
            Self::Operation(error) => error.hint(),
            Self::Event(error) => error.hint(),
            Self::Reservation(error) => error.hint(),
            Self::Serialization(_) => "inspect the retained layout and operation records",
            Self::InvalidJson(_) | Self::InvalidParams { .. } => {
                "send only the documented fields with explicit stable identities"
            }
            Self::NotFound { .. } => "read the current layout and name an existing identity",
            Self::InvalidGeometry(_) => "choose a value inside the returned range and step",
            Self::RevisionMismatch { .. } => "read the current layout and recompute the edit",
            Self::GroupScopeRequired { .. } => {
                "repeat with group scope after reviewing every affected identity"
            }
            Self::LimitExceeded => "close unused workspaces, tabs or panes first",
            Self::LauncherUnavailable => "submit the launch through a terminal-capable endpoint",
        }
    }

    /// Wire error retained in the operation ledger.
    pub fn data(&self) -> ErrorData {
        let error = BoundaryError::domain(ErrorKind::from(self.kind().to_owned()), false)
            .with_hint(self.hint());
        let error = match self {
            Self::NotFound { resource } => match target(resource) {
                Some(id) => error.with_target(id),
                None => error,
            },
            _ => error,
        };
        error.data().clone()
    }
}

fn target(resource: &ResourceId) -> Option<TargetId> {
    let id = match resource {
        ResourceId::Workspace(id) => id.to_string(),
        ResourceId::Tab(id) => id.to_string(),
        ResourceId::Pane(id) => id.to_string(),
        ResourceId::Split(id) => id.to_string(),
        _ => return None,
    };
    Some(TargetId::new(id))
}
