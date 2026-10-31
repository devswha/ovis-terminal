//! The single writer of a connection: replies, notifications, the upgrade and `$/closing`.

use ovis_terminal_protocol::rpc::RpcReply;
use ovis_terminal_protocol::wire::{Channel, Frame, FrameKind};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use super::ConnectionError;

/// Outbound records queued per connection before the reader stops reading.
pub(super) const OUTBOUND_CAPACITY: usize = 256;

/// The reason a server closes a connection, sent as the `$/closing` params.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct Closing {
    reason: &'static str,
    hint: &'static str,
}

impl Closing {
    pub(super) const SERVER_SHUTDOWN: Self = Self {
        reason: "server_shutdown",
        hint: "reconnect once the server is running again",
    };

    pub(super) fn of(error: &ConnectionError) -> Self {
        Self {
            reason: error.kind(),
            hint: error.hint(),
        }
    }
}

#[derive(Debug)]
pub(super) enum Outbound {
    Reply(RpcReply),
    Notification { method: String, params: Value },
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

/// The connection was closed before a notification could be queued.
#[derive(Debug, Error)]
#[error("the connection is closed")]
#[non_exhaustive]
pub struct ConnectionClosed;

impl ConnectionClosed {
    /// The stable machine-readable kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        "connection_closed"
    }

    /// Recovery guidance for the kind.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        "stop producing for this connection; the client reconnects"
    }
}

/// Queues server notifications on the connection that carried a request.
#[derive(Debug, Clone)]
pub struct Notifier {
    outbound: mpsc::Sender<Outbound>,
}

impl Notifier {
    pub(super) const fn new(outbound: mpsc::Sender<Outbound>) -> Self {
        Self { outbound }
    }

    /// Queues `method` with `params`, waiting while the outbound queue is full.
    ///
    /// # Errors
    ///
    /// [`ConnectionClosed`] once the connection has closed.
    pub async fn notify(&self, method: &str, params: Value) -> Result<(), ConnectionClosed> {
        let notification = Outbound::Notification {
            method: method.to_owned(),
            params,
        };
        self.outbound
            .send(notification)
            .await
            .map_err(|_| ConnectionClosed)
    }
}

/// Writes queued records in order until the queue closes or `$/closing` is written.
pub(super) async fn write_loop<W: AsyncWrite + Unpin>(
    mut writer: W,
    mut queue: mpsc::Receiver<Outbound>,
) -> Result<(), ConnectionError> {
    let mut framed = false;
    while let Some(item) = queue.recv().await {
        let (json, last) = match &item {
            Outbound::Upgrade => {
                framed = true;
                continue;
            }
            Outbound::End => break,
            Outbound::Reply(reply) => (serde_json::to_vec(reply)?, false),
            Outbound::Notification { method, params } => (record(method, params)?, false),
            Outbound::Closing(closing) => (record("$/closing", closing)?, true),
        };
        let bytes = if framed {
            Frame::new(Channel::CONTROL, FrameKind::Data, json)?.encode()?
        } else {
            let mut line = json;
            line.push(b'\n');
            line
        };
        writer.write_all(&bytes).await?;
        if last {
            break;
        }
    }
    writer.shutdown().await?;
    Ok(())
}

fn record<P: Serialize>(method: &str, params: &P) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&NotificationRecord {
        jsonrpc: "2.0",
        method,
        params,
    })
}
