//! Failures of orphan delivery, receipt and acknowledgement.

use ovis_terminal_protocol::domain::machine_actor::NotificationId;

use crate::actors::ActorError;

/// The durable outbox failed; its own kind and hint are kept.
#[derive(Debug, thiserror::Error)]
#[error("the orphan outbox failed")]
#[non_exhaustive]
pub struct OutboxError {
    kind: &'static str,
    hint: &'static str,
    #[source]
    source: Box<dyn std::error::Error + Send + Sync>,
}

impl OutboxError {
    /// A store failure of `kind` with `hint`.
    pub fn new(
        (kind, hint): (&'static str, &'static str),
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Self {
            kind,
            hint,
            source: source.into(),
        }
    }

    /// Stable machine token of the underlying failure.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        self.kind
    }

    /// Recovery guidance of the underlying failure.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        self.hint
    }
}

/// Why an orphan notice was not delivered, received or acknowledged.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OrphanError {
    /// No pending or recently settled notice has this id.
    #[error("no orphan notice {} is pending", notification_id.as_str())]
    NotFound {
        /// The id named.
        notification_id: NotificationId,
    },
    /// The id names a notice of another parent incarnation, or of another machine.
    #[error("orphan notice {} belongs to another parent incarnation", notification_id.as_str())]
    StaleTarget {
        /// The id named.
        notification_id: NotificationId,
    },
    /// The receiving machine answered with an error; the notice stays pending.
    #[error("the receiving machine refused the orphan notice")]
    Refused,
    /// The receiving machine acknowledged another notice than the one sent.
    #[error("the receiving machine acknowledged another notice")]
    Mismatch,
    /// The machine's actor did not deliver the notice; it stays pending.
    #[error("the orphan notice was not delivered")]
    Actor(#[from] ActorError),
    /// The durable outbox failed.
    #[error("the orphan outbox failed")]
    Outbox(#[from] OutboxError),
    /// A notice or answer did not encode or decode.
    #[error("an orphan record did not encode or decode")]
    Codec(#[from] serde_json::Error),
}

impl OrphanError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "not_found",
            Self::StaleTarget { .. } => "stale_target",
            Self::Refused => "remote_refused",
            Self::Mismatch | Self::Codec(_) => "remote_incompatible",
            Self::Actor(error) => error.kind(),
            Self::Outbox(error) => error.kind(),
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "acknowledge an id a pending orphan notice carried",
            Self::StaleTarget { .. } => {
                "echo the notice's parent incarnation; a reused parent address is another parent"
            }
            Self::Refused => "inspect the receiving machine; the notice is replayed on reconnect",
            Self::Mismatch | Self::Codec(_) => {
                "update the remote runtime to this protocol generation"
            }
            Self::Actor(error) => error.hint(),
            Self::Outbox(error) => error.hint(),
        }
    }
}
