//! Typed operation boundary failures.

use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::server::OpRecord;
use ovis_terminal_protocol::schema::scalar::ByteCount;
use ovis_terminal_store::StoreError;

use super::ParamsBound;
use crate::events::EventError;

/// A rejected operation action, preserving its prior receipt when one exists.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OperationError {
    /// The clock could not supply an operation timestamp.
    #[error("the system clock predates the operation clock epoch")]
    Clock(#[from] std::time::SystemTimeError),
    /// Authority lapsed at a transactional effect boundary.
    #[error(transparent)]
    Authority(#[from] ovis_terminal_authz::AuthzError),
    /// A resource identity could not be reserved in the operation's commit unit.
    #[error(transparent)]
    Reservation(#[from] crate::registry::ReservationError),
    /// JSON encoding or persisted record decoding failed.
    #[error("the operation record could not be encoded or decoded")]
    Serialization(#[from] serde_json::Error),
    /// The coordinator rejected the durable transition.
    #[error(transparent)]
    Event(#[from] EventError),
    /// A transition failed after its operation identity was known.
    #[error("the durable operation transition could not be acknowledged")]
    Transition {
        /// Identity clients must inspect before retrying an effect.
        op_id: OpId,
        /// Coordinator failure, retained without string conversion.
        #[source]
        source: EventError,
    },
    /// The UUID parser rejected the operation identity.
    #[error("the operation identity is not a UUID")]
    InvalidId(#[source] uuid::Error),
    /// A request violates the operation boundary contract.
    #[error("the request must name a canonical UUIDv7 and a method parameter object")]
    InvalidParams,
    /// A reused identity names different method parameters.
    #[error("the operation identity already names different work")]
    Reused(Box<OpRecord>),
    /// The identity is below the retained acceptance floor.
    #[error("the operation identity predates the retained acceptance window")]
    Expired,
    /// No record exists for this principal and lifetime.
    #[error("no retained operation matches this principal and runtime")]
    NotFound,
    /// No execution permission can be granted for this state.
    #[error("the retained operation cannot start again")]
    Settled(Box<OpRecord>),
    /// Too much retained work is already admitted.
    #[error("the retained operation capacity is exhausted")]
    Capacity,
    /// The resulting receipt or holder evidence exceeds its byte bound.
    #[error("the operation payload exceeds its byte bound")]
    TooLarge,
    /// Canonical params exceed the bound of their method.
    #[error("canonical params of {} bytes exceed the {}-byte bound", actual.get(), bound.bytes().get())]
    ParamsTooLarge {
        /// The method's bound.
        bound: ParamsBound,
        /// Length of the canonical params encoding.
        actual: ByteCount,
    },
    /// Persisted data does not match its index or runtime.
    #[error("the operation ledger does not match its committed identity")]
    InvalidLedger,
    /// Evidence names a different terminal or process.
    #[error("the holder evidence names a different process")]
    EvidenceMismatch,
}

impl OperationError {
    /// Stable machine token, never inferred from Display text.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Clock(_) => "clock_unavailable",
            Self::Authority(error) => error.kind(),
            Self::Reservation(error) => error.kind(),
            Self::Serialization(_) => "persistence_failed",
            Self::Event(error) | Self::Transition { source: error, .. } => error.kind(),
            Self::InvalidId(_) | Self::InvalidParams => "invalid_params",
            Self::Reused(_) => "op_reused",
            Self::Expired => "op_expired",
            Self::NotFound => "not_found",
            Self::Settled(record) => match &record.state {
                ovis_terminal_protocol::schema::scalar::OpenEnum::Known(
                    ovis_terminal_protocol::schema::methods::server::OpState::OutcomeUnknown,
                ) => "outcome_unknown",
                ovis_terminal_protocol::schema::scalar::OpenEnum::Known(
                    ovis_terminal_protocol::schema::methods::server::OpState::Cancelled,
                ) => "cancelled",
                _ => "already_started",
            },
            Self::Capacity => "backpressure",
            Self::TooLarge | Self::ParamsTooLarge { .. } => "too_large",
            Self::InvalidLedger => "operation_store_invalid",
            Self::EvidenceMismatch => "stale_target",
        }
    }

    /// Recovery guidance suitable for structured boundary errors.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Clock(_) => "restore a clock at or after the Unix epoch",
            Self::Authority(error) => error.hint(),
            Self::Reservation(error) => error.hint(),
            Self::Event(error) | Self::Transition { source: error, .. } => error.hint(),
            Self::Serialization(_) | Self::InvalidLedger => {
                "inspect retained operation and recovery evidence"
            }
            Self::InvalidId(_) | Self::InvalidParams => {
                "supply a canonical UUIDv7 and validated method parameters"
            }
            Self::Reused(_) => {
                "inspect the original operation; use a new identity only for new work"
            }
            Self::Expired => "inspect external effects before explicitly submitting new work",
            Self::NotFound => "query an operation accepted by this principal in this runtime",
            Self::Settled(_) => {
                "inspect the retained outcome; never automatically replay uncertain effects"
            }
            Self::Capacity => {
                "wait for the advertised retention floor to advance before admitting new work"
            }
            Self::TooLarge => "bound output and parameters before submitting the operation",
            Self::ParamsTooLarge { bound, .. } => bound.hint(),
            Self::EvidenceMismatch => {
                "join holder evidence only to its original terminal and process incarnation"
            }
        }
    }

    /// Prior outcome attached to a reuse or repeated-start rejection.
    pub const fn receipt(&self) -> Option<&OpRecord> {
        match self {
            Self::Reused(record) | Self::Settled(record) => Some(record),
            _ => None,
        }
    }

    /// Identity to inspect when a durable transition could not be acknowledged.
    pub const fn op_id(&self) -> Option<&OpId> {
        match self {
            Self::Transition { op_id, .. } => Some(op_id),
            Self::Reused(record) | Self::Settled(record) => Some(&record.op_id),
            _ => None,
        }
    }

    pub(super) const fn is_conflict(&self) -> bool {
        matches!(
            self,
            Self::Event(EventError::Store(StoreError::Conflict { .. }))
                | Self::Transition {
                    source: EventError::Store(StoreError::Conflict { .. }),
                    ..
                }
        )
    }
}
