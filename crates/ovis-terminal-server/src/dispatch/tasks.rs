//! Operation tasks retain bounded ownership after their connection's RPC has settled.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use ovis_terminal_authz::{PrincipalId, Session};
use ovis_terminal_core::ops::Request;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind, OpId};
use ovis_terminal_protocol::schema::limits::Limits;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tokio::task::{Id, JoinSet};

use super::DispatchError;
use super::deadline::Response;
use super::queue::Queued;
use super::reservation::Reservation;

pub(super) struct Active {
    pub principal: PrincipalId,
    pub request: Arc<Request>,
    pub cancel: watch::Sender<bool>,
    pub response: Response,
    pub reservation: Arc<Reservation>,
    pub queued: Queued,
}

/// One connection's in-flight budget and its bounded admission queue.
pub(super) struct Connection {
    session: Weak<Session>,
    in_flight: Arc<Semaphore>,
    queued: Arc<Semaphore>,
}

/// A request's in-flight slot and its place in the connection's admission queue.
pub(super) struct Permits {
    pub in_flight: OwnedSemaphorePermit,
    pub queued: OwnedSemaphorePermit,
}

#[derive(Default)]
pub(super) struct Tasks {
    pub set: JoinSet<()>,
    pub active: HashMap<Id, Active>,
    pub connections: Vec<Connection>,
    pub closed: bool,
}

impl Tasks {
    pub(super) fn joined(
        &self,
        principal: &PrincipalId,
        request: &Request,
    ) -> Result<Option<(Response, Queued)>, BoundaryError> {
        if self.closed {
            return Err(DispatchError::Closed.boundary());
        }
        let active = self.active.values().find(|active| {
            &active.principal == principal && active.request.op_id() == request.op_id()
        });
        match active {
            Some(active) if active.request.as_ref() == request => {
                Ok(Some((active.response.clone(), active.queued.clone())))
            }
            Some(_) => Err(
                BoundaryError::domain(ErrorKind::from("op_reused".to_owned()), false)
                    .with_hint(
                        "inspect the original operation; use a new identity only for new work",
                    )
                    .with_op_id(request.op_id().clone()),
            ),
            None => Ok(None),
        }
    }

    pub(super) fn signal(&self, principal: &PrincipalId, op_id: &OpId) {
        for active in self.active.values() {
            if &active.principal == principal && active.request.op_id() == op_id {
                active.cancel.send_replace(true);
            }
        }
    }

    pub(super) fn take_running(&mut self) -> Self {
        Self {
            set: std::mem::take(&mut self.set),
            active: std::mem::take(&mut self.active),
            ..Self::default()
        }
    }

    pub(super) async fn join(mut self) -> Result<(), DispatchError> {
        let mut failed = None;
        while let Some(result) = self.set.join_next_with_id().await {
            match result {
                Ok((id, ())) => {
                    self.active.remove(&id);
                }
                Err(error) => {
                    self.active.remove(&error.id());
                    failed.get_or_insert(DispatchError::Join(error));
                }
            }
        }
        failed.map_or(Ok(()), Err)
    }

    pub(super) fn reap(&mut self) -> Result<(), DispatchError> {
        while let Some(result) = self.set.try_join_next_with_id() {
            match result {
                Ok((id, ())) => {
                    self.active.remove(&id);
                }
                Err(error) => {
                    self.active.remove(&error.id());
                    return Err(error.into());
                }
            }
        }
        Ok(())
    }

    pub(super) fn permit(&mut self, session: &Arc<Session>) -> Result<Permits, DispatchError> {
        self.reap()?;
        if self.closed {
            return Err(DispatchError::Closed);
        }
        let limits = Limits::v1();
        let limit = usize::try_from(limits.max_in_flight_requests.get())?;
        let bound = usize::try_from(limits.queued_operations.get())?;
        self.connections.retain(|connection| {
            connection.session.strong_count() > 0
                || connection.in_flight.available_permits() < limit
        });
        let index = self.connections.iter().position(|connection| {
            connection
                .session
                .upgrade()
                .is_some_and(|s| Arc::ptr_eq(&s, session))
        });
        let connection = if let Some(connection) = index.and_then(|at| self.connections.get(at)) {
            connection
        } else {
            self.connections.push(Connection {
                session: Arc::downgrade(session),
                in_flight: Arc::new(Semaphore::new(limit)),
                queued: Arc::new(Semaphore::new(bound)),
            });
            self.connections.last().ok_or(DispatchError::Closed)?
        };
        let in_flight = Arc::clone(&connection.in_flight)
            .try_acquire_owned()
            .map_err(|_| DispatchError::Backpressure)?;
        let queued = Arc::clone(&connection.queued)
            .try_acquire_owned()
            .map_err(|_| DispatchError::QueueFull {
                bound: limits.queued_operations,
            })?;
        Ok(Permits { in_flight, queued })
    }
}
