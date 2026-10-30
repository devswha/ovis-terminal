//! Typed failures while parsing identities and display names.

use super::resources::RuntimeId;

/// A boundary failure while parsing a typed identity or resource name.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IdentityError {
    /// The value does not use the canonical family-prefixed identity form.
    #[error("the identity does not use its canonical representation")]
    InvalidIdentity,
    /// The identity belongs to a different resource family.
    #[error("the identity belongs to {actual}, not {expected}")]
    WrongFamily {
        /// The expected identity family.
        expected: &'static str,
        /// The supplied identity family.
        actual: String,
    },
    /// An identity or generation cannot be zero.
    #[error("zero is not a valid identity or generation")]
    Zero,
    /// A decimal identity or generation is outside the supported range.
    #[error("the decimal identity or generation exceeds the supported range")]
    Overflow,
    /// A persistent generation counter cannot advance without wrapping.
    #[error("the persistent generation counter is exhausted")]
    CounterOverflow,
    /// A display name contains unsupported characters or has an invalid length.
    #[error("the display name is empty, too long, or contains reserved characters")]
    InvalidName,
    /// A resource belongs to another runtime lifetime.
    #[error("the resource belongs to a different runtime lifetime")]
    CrossLifetime {
        /// The runtime lifetime requested by the caller.
        expected: RuntimeId,
        /// The runtime lifetime of the resource.
        actual: RuntimeId,
    },
}

impl IdentityError {
    /// The stable machine-readable error kind.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::InvalidIdentity | Self::WrongFamily { .. } | Self::Zero | Self::Overflow => {
                "invalid_identity"
            }
            Self::CounterOverflow => "counter_overflow",
            Self::InvalidName => "invalid_name",
            Self::CrossLifetime { .. } => "cross_lifetime",
        }
    }

    /// Recovery guidance for the error kind.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::InvalidIdentity | Self::WrongFamily { .. } | Self::Zero | Self::Overflow => {
                "supply a canonical nonzero identity from the required resource family"
            }
            Self::CounterOverflow => {
                "start a new runtime lifetime before allocating more generations"
            }
            Self::InvalidName => "use 1 to 64 ASCII letters, digits, dots, underscores, or hyphens",
            Self::CrossLifetime { .. } => {
                "refresh the resource binding from the requested runtime lifetime"
            }
        }
    }
}
