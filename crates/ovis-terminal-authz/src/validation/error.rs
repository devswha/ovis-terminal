//! Why a caller-named path was refused.

use std::io;

/// A refused or unreadable root-scoped path; it carries the field, never the path text.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PathError {
    /// The authorized root itself cannot be resolved.
    #[error("the authorized root cannot be resolved")]
    Root(#[source] io::Error),
    /// The path does not exist below the root.
    #[error("{field} names nothing below its authorized root")]
    NotFound {
        /// The offending parameter.
        field: &'static str,
    },
    /// Following the path's symlinks leaves the authorized root.
    #[error("{field} resolves outside its authorized root")]
    Escaped {
        /// The offending parameter.
        field: &'static str,
    },
    /// The path resolved to something else since it was authorized.
    #[error("{field} changed after it was authorized")]
    Replaced {
        /// The offending parameter.
        field: &'static str,
    },
    /// Resolving the path failed for another reason.
    #[error("{field} cannot be resolved")]
    Io {
        /// The offending parameter.
        field: &'static str,
        /// The underlying failure.
        #[source]
        source: io::Error,
    },
}

impl PathError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Root(_) => "store_unavailable",
            Self::NotFound { .. } => "path_not_found",
            Self::Escaped { .. } | Self::Replaced { .. } => "forbidden",
            Self::Io { .. } => "internal",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Root(_) => "ask the runtime owner to configure an existing root",
            Self::NotFound { .. } => "list the root and name an existing entry",
            Self::Escaped { .. } => "name a path that stays inside the authorized root",
            Self::Replaced { .. } => "resolve the path again; it was replaced after authorization",
            Self::Io { .. } => "inspect the root's permissions and retry",
        }
    }

    /// The offending parameter, when the failure is the caller's path.
    #[must_use]
    pub const fn field_path(&self) -> Option<&'static str> {
        match self {
            Self::Root(_) => None,
            Self::NotFound { field }
            | Self::Escaped { field }
            | Self::Replaced { field }
            | Self::Io { field, .. } => Some(field),
        }
    }
}
