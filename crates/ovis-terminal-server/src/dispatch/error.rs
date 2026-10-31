//! Failures at registration, admission and the owned execution boundary.

use ovis_terminal_authz::AuthzError;
use ovis_terminal_core::ops::OperationError;
use ovis_terminal_core::registry::TargetResolutionError;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind, TargetCandidate};
use ovis_terminal_protocol::schema::ParamsError;
use ovis_terminal_protocol::schema::ids::{MachineId, RuntimeLifetimeId, TerminalId};

/// A dispatch failure; inner diagnostics are retained as sources, not sent as raw prose.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DispatchError {
    /// An effect reported an I/O failure; dispatch preserves uncertain delivery.
    #[error("the domain effect could not complete its I/O")]
    Io(#[from] std::io::Error),
    /// Method parameters did not match their closed type.
    #[error("the request parameters could not be parsed")]
    Params(#[from] ParamsError),
    /// A target did not resolve uniquely.
    #[error("the explicit target could not be resolved")]
    Target(#[from] TargetResolutionError),
    /// The principal cannot perform this action.
    #[error("the authority refused this action")]
    Authority(#[from] AuthzError),
    /// Durable operation transition failed.
    #[error("the operation ledger refused this transition")]
    Operation(#[from] OperationError),
    /// Encoding a typed result failed.
    #[error("the typed result could not be encoded")]
    Encode(#[from] serde_json::Error),
    /// An owned task failed.
    #[error("an owned dispatch task failed")]
    Join(#[from] tokio::task::JoinError),
    /// A registration does not name one unique implemented catalog method.
    #[error("the method is absent from the catalog or already registered")]
    Registration,
    /// Work exceeded its bounded admission budget.
    #[error("the dispatch admission budget is exhausted")]
    Backpressure,
    /// Dispatch has shut down.
    #[error("the dispatcher has stopped accepting work")]
    Closed,
    /// The wall clock cannot be represented by the ledger.
    #[error("the clock cannot represent an operation timestamp")]
    Clock(#[from] std::time::SystemTimeError),
    /// A platform counter cannot represent the requested quantity.
    #[error("the counter does not fit the platform representation")]
    Range(#[from] std::num::TryFromIntError),
    /// The monotonic clock cannot represent the deadline.
    #[error("the monotonic clock cannot represent the dispatch deadline")]
    DeadlineOverflow,
}

impl DispatchError {
    /// Stable machine token.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Params(error) => error.kind(),
            Self::Target(error) => error.kind(),
            Self::Authority(error) => error.kind(),
            Self::Operation(error) => error.kind(),
            Self::Encode(_)
            | Self::Join(_)
            | Self::Clock(_)
            | Self::Io(_)
            | Self::Range(_)
            | Self::DeadlineOverflow => "internal",
            Self::Registration => "method_not_found",
            Self::Backpressure => "backpressure",
            Self::Closed => "unavailable",
        }
    }

    /// Recovery guidance without exposing parser or credential diagnostics.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Params(error) => error.hint(),
            Self::Target(error) => error.hint(),
            Self::Authority(error) => error.hint(),
            Self::Operation(error) => error.hint(),
            Self::Encode(_) | Self::Join(_) | Self::Io(_) => {
                "inspect the operation before retrying"
            }
            Self::Registration => "register an implemented method from the published catalog",
            Self::Backpressure => "inspect or cancel outstanding operations before retrying",
            Self::Closed => "connect to an available runtime",
            Self::Clock(_) | Self::DeadlineOverflow => "correct the runtime clock",
            Self::Range(_) => "use a quantity supported by this platform",
        }
    }

    pub(super) fn boundary(&self) -> BoundaryError {
        if let Self::Params(error) = self {
            return BoundaryError::invalid_params(Some(error.field_path()));
        }
        if let Self::Operation(OperationError::Settled(record)) = self
            && let Some(data) = &record.error
        {
            return BoundaryError::Domain {
                data: Box::new(data.clone()),
            };
        }
        if matches!(
            self,
            Self::Operation(OperationError::InvalidParams | OperationError::InvalidId(_))
        ) {
            return BoundaryError::invalid_params(Some("params.op_id"));
        }
        let mut error = BoundaryError::domain(
            ErrorKind::from(self.kind().to_owned()),
            matches!(self, Self::Backpressure),
        )
        .with_hint(self.hint());
        if let Self::Target(source) = self
            && let Some(candidates) = source.candidates()
        {
            error = error.with_candidates(
                candidates
                    .iter()
                    .map(|candidate| {
                        TargetCandidate::new(
                            MachineId::new(candidate.machine_id().to_string()),
                            RuntimeLifetimeId::new(candidate.runtime_id().to_string()),
                            TerminalId::new(candidate.terminal_id().to_string()),
                        )
                    })
                    .collect(),
            );
        }
        if let Self::Operation(source) = self
            && let Some(op_id) = source.op_id()
        {
            error = error.with_op_id(op_id.clone());
        }
        error
    }
}
