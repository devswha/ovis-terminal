//! Drives one connection: reads, bounded concurrent requests, ordered replies and close.
//!
//! Every task the connection starts lives in a `JoinSet` owned here, so dropping or
//! shutting down the connection cancels and joins them; nothing outlives it.

use std::collections::HashMap;
use std::sync::Arc;

use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::rpc::{InFlightRequests, RequestId, RpcFailure, RpcReply, RpcSuccess};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::sync::{Semaphore, mpsc, watch};
use tokio::task::{self, JoinError, JoinSet};

use super::core::{ConnectionCore, Step};
use super::outbound::{Closing, OUTBOUND_CAPACITY, Outbound, write_loop};
use super::{Call, ConnectionError, Notifier, RequestHandler};

const READ_CHUNK: usize = 64 * 1024;

/// What one connection needs beyond its stream and state machine.
#[derive(Debug)]
pub(super) struct DriverContext<H> {
    pub(super) handler: Arc<H>,
    pub(super) shutdown: watch::Receiver<bool>,
    pub(super) max_in_flight: usize,
}

enum End {
    PeerClosed,
    Shutdown,
    Failed(ConnectionError),
}

struct Requests<H> {
    handler: Arc<H>,
    outbound: mpsc::Sender<Outbound>,
    tasks: JoinSet<RpcReply>,
    ids: HashMap<task::Id, RequestId>,
    in_flight: InFlightRequests,
    permits: Arc<Semaphore>,
}

/// Serves one connection until the peer closes, the server shuts down or a failure.
pub(super) async fn drive<S, H>(
    stream: S,
    mut core: ConnectionCore,
    mut context: DriverContext<H>,
) -> Result<(), ConnectionError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
    H: RequestHandler,
{
    let (mut reader, writer) = tokio::io::split(stream);
    let (outbound, queue) = mpsc::channel(OUTBOUND_CAPACITY);
    let mut writers = JoinSet::new();
    writers.spawn(write_loop(writer, queue));
    let mut requests = Requests {
        handler: Arc::clone(&context.handler),
        outbound: outbound.clone(),
        tasks: JoinSet::new(),
        ids: HashMap::new(),
        in_flight: InFlightRequests::default(),
        permits: Arc::new(Semaphore::new(context.max_in_flight)),
    };
    let mut buffer = vec![0_u8; READ_CHUNK];
    let end = loop {
        tokio::select! {
            biased;
            _ = context.shutdown.changed() => break End::Shutdown,
            Some(joined) = requests.tasks.join_next_with_id(), if !requests.tasks.is_empty() => {
                if let Err(error) = requests.settle(joined).await {
                    break End::Failed(error);
                }
            }
            read = reader.read(&mut buffer) => {
                let bytes = match read {
                    Ok(0) => break End::PeerClosed,
                    Ok(count) => match buffer.get(..count) {
                        Some(bytes) => bytes,
                        None => break End::Failed(ConnectionError::Io(
                            std::io::ErrorKind::InvalidData.into(),
                        )),
                    },
                    Err(error) if peer_gone(&error) => break End::PeerClosed,
                    Err(error) => break End::Failed(error.into()),
                };
                if let Some(end) = requests.apply(core.feed(bytes)).await {
                    break end;
                }
            }
        }
    };
    let result = requests.finish(end, &mut context.shutdown).await;
    // Handlers may keep notifiers past the connection, so the writer is ended explicitly.
    // A refused send means the writer already stopped; its own result is reported below.
    match requests.send(Outbound::End).await {
        Ok(()) | Err(_) => {}
    }
    drop(requests);
    drop(outbound);
    let written = match writers.join_next().await {
        Some(Ok(Err(ConnectionError::Io(error)))) if peer_gone(&error) => Ok(()),
        Some(Ok(written)) => written,
        Some(Err(error)) => Err(ConnectionError::Io(std::io::Error::other(error))),
        None => Ok(()),
    };
    result.and(written)
}

impl<H: RequestHandler> Requests<H> {
    async fn apply(&mut self, steps: Vec<Step>) -> Option<End> {
        for step in steps {
            let queued = match step {
                Step::Reply(reply) => self.send(Outbound::Reply(reply)).await,
                Step::Upgrade => self.send(Outbound::Upgrade).await,
                Step::Dispatch { request, session } => {
                    let notifier = Notifier::new(self.outbound.clone());
                    self.dispatch(Call {
                        request,
                        session,
                        notifier,
                    })
                    .await
                }
                Step::Close(error) => return Some(End::Failed(error)),
            };
            if let Err(error) = queued {
                return Some(End::Failed(error));
            }
        }
        None
    }

    async fn dispatch(&mut self, call: Call) -> Result<(), ConnectionError> {
        let id = call.request.id().clone();
        if let Err(error) = self.in_flight.begin(id.clone()) {
            return self.send(failure(id, &error)).await;
        }
        let Ok(permit) = Arc::clone(&self.permits).try_acquire_owned() else {
            let busy = BoundaryError::domain(ErrorKind::from("backpressure".to_owned()), true)
                .with_hint("wait for an in-flight request to settle, then retry");
            let reply = RpcReply::Failure(RpcFailure::new(id.clone(), &busy));
            let settled = self
                .in_flight
                .complete(&id, reply)
                .map_err(ConnectionError::Correlation)?;
            return self.send(Outbound::Reply(settled)).await;
        };
        let handler = Arc::clone(&self.handler);
        let task = self.tasks.spawn(async move {
            let id = call.request.id().clone();
            let reply = match handler.call(call).await {
                Ok(result) => RpcReply::Success(RpcSuccess::new(id, result)),
                Err(error) => RpcReply::Failure(RpcFailure::new(id, &error)),
            };
            drop(permit);
            reply
        });
        self.ids.insert(task.id(), id);
        Ok(())
    }

    async fn settle(
        &mut self,
        joined: Result<(task::Id, RpcReply), JoinError>,
    ) -> Result<(), ConnectionError> {
        let (task, reply) = match joined {
            Ok((task, reply)) => (task, Some(reply)),
            Err(error) => (error.id(), None),
        };
        let id = self.ids.remove(&task).ok_or_else(|| {
            ConnectionError::Correlation(BoundaryError::unknown_or_settled_request_id())
        })?;
        let reply = reply.unwrap_or_else(|| {
            let panicked = BoundaryError::domain(ErrorKind::Internal, false);
            RpcReply::Failure(RpcFailure::new(id.clone(), &panicked))
        });
        let settled = self
            .in_flight
            .complete(&id, reply)
            .map_err(ConnectionError::Correlation)?;
        self.send(Outbound::Reply(settled)).await
    }

    async fn finish(
        &mut self,
        end: End,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Result<(), ConnectionError> {
        let closing = match end {
            End::PeerClosed => return self.drain_after_eof(shutdown).await,
            End::Shutdown => Closing::SERVER_SHUTDOWN,
            End::Failed(error) => {
                self.tasks.shutdown().await;
                let closing = Outbound::Closing(Closing::of(&error));
                // A peer that is already gone cannot read `$/closing`; the original failure
                // is the one reported either way.
                return match self.send(closing).await {
                    Ok(()) | Err(_) => Err(error),
                };
            }
        };
        self.tasks.shutdown().await;
        self.send(Outbound::Closing(closing)).await
    }

    /// The peer stopped sending but may still read: settle what it already asked for.
    async fn drain_after_eof(
        &mut self,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Result<(), ConnectionError> {
        while !self.tasks.is_empty() {
            tokio::select! {
                biased;
                _ = shutdown.changed() => {
                    self.tasks.shutdown().await;
                    return self.send(Outbound::Closing(Closing::SERVER_SHUTDOWN)).await;
                }
                Some(joined) = self.tasks.join_next_with_id() => self.settle(joined).await?,
            }
        }
        Ok(())
    }

    async fn send(&self, item: Outbound) -> Result<(), ConnectionError> {
        self.outbound
            .send(item)
            .await
            .map_err(|_| ConnectionError::Io(std::io::Error::from(std::io::ErrorKind::BrokenPipe)))
    }
}

/// A peer that hung up ends its connection normally; it is not a server failure.
fn peer_gone(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::NotConnected
    )
}

fn failure(id: RequestId, error: &BoundaryError) -> Outbound {
    Outbound::Reply(RpcReply::Failure(RpcFailure::new(id, error)))
}
