//! The subscription boundary error, its stable kinds and recovery hints.

use crate::events::EventError;
use crate::registry::IdentityError;

/// Why a subscription operation was refused or could not complete.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SubscriptionError {
    /// The commit coordinator failed underneath the subscription layer.
    #[error(transparent)]
    Event(#[from] EventError),
    /// A filter list names nothing, so it could never match.
    #[error("the filter field `{field}` lists nothing to match")]
    EmptySelector {
        /// Offending filter field.
        field: &'static str,
    },
    /// A filter field holds an identity that does not parse.
    #[error("the filter field `{field}` holds a malformed identity")]
    InvalidIdentity {
        /// Offending filter field.
        field: &'static str,
        /// Parse failure.
        #[source]
        source: IdentityError,
    },
    /// A selection needs a registry lookup this layer cannot perform exactly.
    #[error("the filter field `{field}` cannot be evaluated against committed events")]
    UnresolvedSelector {
        /// Offending filter field.
        field: &'static str,
    },
    /// The resume cursor names a runtime lifetime that does not parse.
    #[error("the cursor's runtime lifetime is malformed")]
    InvalidCursor(#[source] IdentityError),
    /// Every subscription slot is in use.
    #[error("every subscription slot is in use")]
    Limit,
    /// No open subscription has this identity.
    #[error("no open subscription has this identity")]
    NotFound,
    /// A queue or session bound is zero, or the heartbeat interval exceeds 15 seconds.
    #[error("subscription bounds must be nonzero and heartbeat at most every 15 seconds")]
    InvalidLimits,
    /// The configured runtime identity is not the coordinator's.
    #[error("the subscription layer was configured for another runtime identity")]
    IdentityMismatch,
    /// The sequence space of this runtime lifetime is exhausted.
    #[error("the event sequence cannot advance further")]
    SequenceExhausted,
    /// The fan-out worker could not be started.
    #[error("the subscription fan-out worker could not start")]
    Spawn(#[source] std::io::Error),
    /// The fan-out worker panicked or shut down.
    #[error("the subscription fan-out worker stopped")]
    Stopped,
}

impl SubscriptionError {
    /// The stable machine-readable kind clients branch on.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Event(source) => source.kind(),
            Self::EmptySelector { .. }
            | Self::InvalidIdentity { .. }
            | Self::UnresolvedSelector { .. } => "filter_invalid",
            Self::InvalidCursor(_) | Self::InvalidLimits | Self::IdentityMismatch => {
                "invalid_params"
            }
            Self::Limit => "subscription_limit",
            Self::NotFound => "not_found",
            Self::SequenceExhausted => "limit_exceeded",
            Self::Spawn(_) => "unavailable",
            Self::Stopped => "coordinator_stopped",
        }
    }

    /// Recovery guidance published with the kind.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Event(source) => source.hint(),
            Self::EmptySelector { .. } => "omit the field to match everything",
            Self::InvalidIdentity { .. } => "send identities exactly as the runtime reported them",
            Self::UnresolvedSelector { .. } => {
                "select by terminal identity, event type or machine identity"
            }
            Self::InvalidCursor(_) => "resume with a cursor this runtime returned",
            Self::Limit => "unsubscribe an unused subscription or update an existing filter",
            Self::NotFound => "subscribe again and resume from the last processed cursor",
            Self::InvalidLimits => "configure nonzero bounds and a heartbeat of at most 15 s",
            Self::IdentityMismatch => {
                "start the subscription layer with the coordinator's identity"
            }
            Self::SequenceExhausted => "restart the runtime to begin a new lifetime",
            Self::Spawn(_) => "free thread resources and start the subscription layer again",
            Self::Stopped => "reconnect and resume from the last processed cursor",
        }
    }
}
