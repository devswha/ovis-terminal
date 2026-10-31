//! Delivers one stream as plain-mode `attach.control` and `attach.data` notifications.
//!
//! Each stream has its own task, so a viewer that grants no credit or whose connection
//! stops reading delays nobody else. When the connection closes, or the stream has
//! made no progress for the stall bound, readiness and the viewer's leases are revoked.
//! A send already queued is always awaited to completion, never dropped mid-way.

use std::sync::Arc;

use base64::Engine as _;
use ovis_terminal_protocol::schema::ids::StreamId;
use ovis_terminal_protocol::schema::records::AttachData;
use ovis_terminal_store::OpId;
use serde_json::Value;
use tokio::sync::Notify;
use tokio::time::sleep;

use super::error::StreamError;
use super::hub::TerminalStreams;
use super::viewer::Delivery;
use crate::conn::{Notifier, NotifyError};

#[derive(Debug)]
pub(super) struct Pump {
    pub hub: Arc<TerminalStreams>,
    pub stream: StreamId,
    pub wake: Arc<Notify>,
    pub notifier: Notifier,
}

enum Sent {
    More,
    Last,
    Gone,
}

fn encode(
    stream: &StreamId,
    delivery: Delivery,
) -> Result<(&'static str, Value, bool), StreamError> {
    match delivery {
        Delivery::Control { record, last } => {
            Ok(("attach.control", serde_json::to_value(&*record)?, last))
        }
        Delivery::Data { offset, bytes } => {
            let data = base64::engine::general_purpose::STANDARD.encode(bytes.as_slice());
            let params = AttachData::new(stream.clone(), offset.wire(), data);
            Ok(("attach.data", serde_json::to_value(params)?, false))
        }
    }
}

fn detach_op() -> OpId {
    OpId::new(uuid::Uuid::now_v7().to_string())
}

impl Pump {
    /// Runs until the stream closes, the connection ends or the viewer is detached.
    pub(super) async fn run(self) -> Result<(), StreamError> {
        let outcome = self.deliver().await;
        let detached = self.detach().await;
        outcome.and(detached)
    }

    async fn deliver(&self) -> Result<(), StreamError> {
        loop {
            let next = match self.hub.next(&self.stream) {
                Ok(next) => next,
                Err(StreamError::StreamNotFound) => return Ok(()),
                Err(error) => return Err(error),
            };
            let Some(delivery) = next else {
                let Some(due) = self.hub.stall(&self.stream)? else {
                    return Ok(());
                };
                tokio::select! {
                    () = self.wake.notified() => {}
                    () = self.notifier.closed() => return Ok(()),
                    () = sleep(due) => {}
                }
                continue;
            };
            match self.send(delivery).await? {
                Sent::More => {}
                Sent::Last | Sent::Gone => return Ok(()),
            }
        }
    }

    /// Sends one notification; if it stays blocked past the stall bound the viewer is
    /// closed and its leases revoked while the queued send is still awaited. Before the
    /// final CLOSE the viewer is detached, so a client that sees CLOSE holds no lease.
    async fn send(&self, delivery: Delivery) -> Result<Sent, StreamError> {
        let (method, params, last) = encode(&self.stream, delivery)?;
        if last {
            // Leases and readiness are revoked and committed before CLOSE is visible.
            self.detach().await?;
        }
        let send = self.notifier.notify(method, params);
        tokio::pin!(send);
        let bound = sleep(self.hub.setup.limits.stall_after);
        tokio::pin!(bound);
        let mut stalled = false;
        loop {
            tokio::select! {
                outcome = &mut send => {
                    return Ok(match outcome {
                        Ok(()) if last => Sent::Last,
                        Ok(()) => Sent::More,
                        Err(NotifyError::Encode(source)) => return Err(source.into()),
                        Err(_) => Sent::Gone,
                    });
                }
                () = &mut bound, if !stalled => {
                    stalled = true;
                    self.hub.force_stall(&self.stream)?;
                    let hub = Arc::clone(&self.hub);
                    let stream = self.stream.clone();
                    tokio::task::spawn_blocking(move || hub.release(&stream, detach_op())).await??;
                }
            }
        }
    }

    async fn detach(&self) -> Result<(), StreamError> {
        let hub = Arc::clone(&self.hub);
        let stream = self.stream.clone();
        tokio::task::spawn_blocking(move || hub.detach(&stream, detach_op())).await?
    }
}
