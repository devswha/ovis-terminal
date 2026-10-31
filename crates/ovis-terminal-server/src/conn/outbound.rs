//! What a connection queues for its writer: replies, notifications, the upgrade and `$/closing`.

use std::sync::Arc;

use ovis_terminal_protocol::rpc::{DecimalU64, RpcReply};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::mpsc;

use super::ConnectionError;
use super::budget::Budget;

/// Outbound records queued per connection before the reader stops reading.
pub(super) const OUTBOUND_CAPACITY: usize = 256;

/// The reason a server closes a connection, sent as the `$/closing` params.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct Closing {
    reason: &'static str,
    hint: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit_bytes: Option<DecimalU64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    queued_bytes: Option<DecimalU64>,
}

impl Closing {
    pub(super) const SERVER_SHUTDOWN: Self = Self {
        reason: "server_shutdown",
        hint: "reconnect once the server is running again",
        limit_bytes: None,
        queued_bytes: None,
    };

    pub(super) fn of(error: &ConnectionError) -> Self {
        let (limit_bytes, queued_bytes) =
            if let ConnectionError::Backpressure { limit, queued } = error {
                (
                    Some(DecimalU64::new(*limit)),
                    Some(DecimalU64::new(*queued)),
                )
            } else {
                (None, None)
            };
        Self {
            reason: error.kind(),
            hint: error.hint(),
            limit_bytes,
            queued_bytes,
        }
    }
}

#[derive(Debug)]
pub(super) enum Outbound {
    /// One encoded JSON-RPC message whose size is reserved in the budget.
    Record(Vec<u8>),
    Upgrade,
    Closing(Closing),
    End,
}

#[derive(Serialize)]
struct NotificationRecord<'a, P> {
    jsonrpc: &'static str,
    method: &'a str,
    params: &'a P,
}

/// Why a notification was not queued.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum NotifyError {
    /// The connection is closed.
    #[error("the connection is closed")]
    Closed,
    /// Queuing it would exceed the connection's unwritten-byte bound; the connection closes.
    #[error("{queued} unwritten bytes are queued; the bound is {limit}")]
    Backpressure {
        /// The connection's unwritten-byte bound.
        limit: u64,
        /// Bytes queued and not yet written when the notification was refused.
        queued: u64,
    },
    /// The notification could not be encoded.
    #[error("the notification could not be encoded")]
    Encode(#[source] serde_json::Error),
}

impl NotifyError {
    /// The stable machine-readable kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Closed => "connection_closed",
            Self::Backpressure { .. } => "backpressure",
            Self::Encode(_) => "internal",
        }
    }

    /// Recovery guidance for the kind.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Closed | Self::Backpressure { .. } => {
                "stop producing for this connection; the client reconnects"
            }
            Self::Encode(_) => "report the notification type; its params do not encode",
        }
    }
}

/// The queue into one connection's writer, guarded by its unwritten-byte budget.
#[derive(Debug, Clone)]
pub(super) struct Outlet {
    pub(super) queue: mpsc::Sender<Outbound>,
    pub(super) budget: Arc<Budget>,
}

impl Outlet {
    /// Encodes and queues one message, reserving its size first.
    pub(super) async fn record<M: Serialize + Sync>(&self, message: &M) -> Result<(), NotifyError> {
        let json = serde_json::to_vec(message).map_err(NotifyError::Encode)?;
        let size = encoded_size(&json);
        self.budget
            .reserve(size)
            .map_err(|overflow| NotifyError::Backpressure {
                limit: overflow.limit,
                queued: overflow.queued,
            })?;
        if self.queue.send(Outbound::Record(json)).await.is_err() {
            self.budget.release(size);
            return Err(NotifyError::Closed);
        }
        Ok(())
    }

    pub(super) async fn reply(&self, reply: &RpcReply) -> Result<(), NotifyError> {
        self.record(reply).await
    }
}

/// Queues server notifications on the connection that carried a request.
#[derive(Debug, Clone)]
pub struct Notifier {
    outlet: Outlet,
}

impl Notifier {
    pub(super) const fn new(outlet: Outlet) -> Self {
        Self { outlet }
    }

    /// Queues `method` with `params`, waiting while the outbound queue is full.
    ///
    /// # Errors
    ///
    /// [`NotifyError::Closed`] once the connection has closed and
    /// [`NotifyError::Backpressure`] when the peer is not reading fast enough.
    pub async fn notify(&self, method: &str, params: Value) -> Result<(), NotifyError> {
        let record = NotificationRecord {
            jsonrpc: "2.0",
            method,
            params: &params,
        };
        self.outlet.record(&record).await
    }

    /// Resolves once the connection's writer has gone, so its owners can revoke
    /// what the connection held even while they have nothing to send.
    pub async fn closed(&self) {
        self.outlet.queue.closed().await;
    }
}

/// The budgeted size of one encoded record. A length beyond `u64` exceeds every bound,
/// so it saturates and the reservation is refused rather than wrapping.
pub(super) fn encoded_size(json: &[u8]) -> u64 {
    u64::try_from(json.len()).unwrap_or(u64::MAX)
}

pub(super) fn record<P: Serialize>(method: &str, params: &P) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&NotificationRecord {
        jsonrpc: "2.0",
        method,
        params,
    })
}
