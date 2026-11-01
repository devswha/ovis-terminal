use crate::recovery::RecoveryError;

/// Why health could not track a consumer or collect a report.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HealthError {
    /// The bounded consumer table is full.
    #[error("{tracked} consumers are already tracked, the table bound")]
    ConsumerCapacity {
        /// Consumers tracked when the request was refused.
        tracked: usize,
    },
    /// Another live gauge already reports for this consumer.
    #[error("another gauge already reports for this consumer")]
    AlreadyTracked,
    /// The incident journal could not be read at boot.
    #[error("the recovery incident journal could not be read")]
    Journal {
        /// The journal failure.
        #[source]
        source: RecoveryError,
    },
    /// An incident could not be made durable; nothing was published.
    #[error("the recovery incident could not be recorded")]
    Record {
        /// The journal failure.
        #[source]
        source: RecoveryError,
    },
    /// The system clock reads before the Unix epoch.
    #[error("the system clock reads before the Unix epoch")]
    Clock,
}

impl HealthError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::ConsumerCapacity { .. } => "consumer_capacity",
            Self::AlreadyTracked => "already_tracked",
            Self::Journal { .. } => "recovery_unreadable",
            Self::Record { .. } => "persistence_failed",
            Self::Clock => "clock_unavailable",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::ConsumerCapacity { .. } => {
                "the consumer still works untracked; server.metrics counts it under consumers_untracked"
            }
            Self::AlreadyTracked => "drop the previous gauge before tracking the consumer again",
            Self::Journal { .. } => {
                "inspect the incident journal in the state directory; it is never rewritten"
            }
            Self::Record { .. } => {
                "check the state directory's disk and permissions, then record the incident again"
            }
            Self::Clock => "fix the host clock; health times are wall-clock milliseconds",
        }
    }
}
