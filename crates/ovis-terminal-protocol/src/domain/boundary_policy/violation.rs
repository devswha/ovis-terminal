//! Refusals at the trust boundary and the payload limits they enforce.

use std::fmt;

use crate::errors::{BoundaryError, ErrorKind, ExceededBound, LimitName};
use crate::schema::scalar::Count;

/// One advertised payload bound, checked before a value is accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PayloadLimit {
    /// Elements of one launch argument vector.
    ArgvElements,
    /// Bytes of one launch argument.
    ArgvElementBytes,
    /// Bytes of all launch arguments together.
    ArgvTotalBytes,
    /// Entries of one launch environment.
    EnvEntries,
    /// Bytes of one environment variable name.
    EnvKeyBytes,
    /// Bytes of one environment variable value.
    EnvValueBytes,
    /// Bytes of one relative path or store reference.
    PathBytes,
}

impl PayloadLimit {
    /// The limit's stable name, as reported in a refusal's `bound`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ArgvElements => "argv_elements",
            Self::ArgvElementBytes => "argv_element_bytes",
            Self::ArgvTotalBytes => "argv_total_bytes",
            Self::EnvEntries => "env_entries",
            Self::EnvKeyBytes => "env_key_bytes",
            Self::EnvValueBytes => "env_value_bytes",
            Self::PathBytes => "path_bytes",
        }
    }

    /// The largest accepted quantity.
    #[must_use]
    pub const fn value(self) -> u64 {
        match self {
            Self::EnvEntries | Self::EnvKeyBytes => 256,
            Self::ArgvElements | Self::PathBytes => 4096,
            Self::ArgvElementBytes | Self::EnvValueBytes => 32 * 1024,
            Self::ArgvTotalBytes => 256 * 1024,
        }
    }

    /// Refuses `actual` above the limit, naming `field`.
    ///
    /// # Errors
    ///
    /// [`BoundaryViolation::TooLarge`] when `actual` exceeds [`PayloadLimit::value`].
    pub fn check(self, field: &'static str, actual: usize) -> Result<(), BoundaryViolation> {
        let actual = u64::try_from(actual).unwrap_or(u64::MAX);
        if actual > self.value() {
            return Err(BoundaryViolation::TooLarge {
                field,
                limit: self,
                actual,
            });
        }
        Ok(())
    }
}

/// Why a value is not even well formed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Malformation {
    /// An argument vector names no program.
    EmptyArgv,
    /// A value holds a NUL byte, which no OS argument, name or path can carry.
    NulByte,
    /// An environment variable name is empty or holds `=`.
    EnvKey,
    /// A path is empty.
    EmptyPath,
}

impl fmt::Display for Malformation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::EmptyArgv => "no program is named",
            Self::NulByte => "a NUL byte is present",
            Self::EnvKey => "a variable name is empty or holds '='",
            Self::EmptyPath => "the path is empty",
        })
    }
}

/// A value refused at the trust boundary; it names the field, never the value.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BoundaryViolation {
    /// The value exceeds an advertised payload limit.
    #[error("{field} exceeds the {} limit of {}", limit.name(), limit.value())]
    TooLarge {
        /// The offending parameter.
        field: &'static str,
        /// The limit reached.
        limit: PayloadLimit,
        /// The refused quantity.
        actual: u64,
    },
    /// A path escapes its authorized root by `..`, an absolute form or a drive prefix.
    #[error("{field} names a path outside its authorized root")]
    Traversal {
        /// The offending parameter.
        field: &'static str,
    },
    /// An environment variable is reserved for the runtime's own bindings.
    #[error("{field} sets a variable reserved for the runtime")]
    ReservedEnvironment {
        /// The offending parameter.
        field: &'static str,
    },
    /// A remote endpoint is not on the allowlist.
    #[error("{field} names an endpoint outside the allowlist")]
    EndpointNotAllowed {
        /// The offending parameter.
        field: &'static str,
    },
    /// The value is not well formed.
    #[error("{field} is malformed: {reason}")]
    Malformed {
        /// The offending parameter.
        field: &'static str,
        /// What is wrong with it.
        reason: Malformation,
    },
}

impl BoundaryViolation {
    /// Stable machine token, the same kind the owning domain reports for this refusal.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::TooLarge { .. } => "too_large",
            Self::Traversal { .. }
            | Self::ReservedEnvironment { .. }
            | Self::EndpointNotAllowed { .. } => "forbidden",
            Self::Malformed { .. } => "invalid_params",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::TooLarge { .. } => "send a value within the limit named in the error bound",
            Self::Traversal { .. } => "name a relative path inside the authorized root",
            Self::ReservedEnvironment { .. } => {
                "drop OVIS_TERMINAL_ variables; the runtime sets its own bindings"
            }
            Self::EndpointNotAllowed { .. } => "ask an administrator to allow this endpoint",
            Self::Malformed { .. } => "correct the field named by field_path",
        }
    }

    /// The offending parameter's path.
    #[must_use]
    pub const fn field_path(&self) -> &'static str {
        match self {
            Self::TooLarge { field, .. }
            | Self::Traversal { field }
            | Self::ReservedEnvironment { field }
            | Self::EndpointNotAllowed { field }
            | Self::Malformed { field, .. } => field,
        }
    }

    /// The wire error: kind, hint, field path and, for an oversized value, the bound.
    #[must_use]
    pub fn to_boundary(&self) -> BoundaryError {
        let error = BoundaryError::domain(ErrorKind::from(self.kind().to_owned()), false)
            .with_hint(self.hint())
            .with_field_path(self.field_path());
        match self {
            Self::TooLarge { limit, .. } => error.with_bound(ExceededBound::new(
                LimitName::new(limit.name().to_owned()),
                Count::new(limit.value()),
            )),
            _ => error,
        }
    }
}
