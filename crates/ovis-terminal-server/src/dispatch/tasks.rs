//! Operation tasks retain bounded ownership after their connection's RPC has settled.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use ovis_terminal_authz::{PrincipalId, Session};
use ovis_terminal_core::ops::Request;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::schema::limits::Limits;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tokio::task::{Id, JoinSet};

use super::DispatchError;
use super::deadline::Response;

pub(super) struct Active {
    pub principal: PrincipalId,
    pub request: Arc<Request>,
    pub cancel: watch::Sender<bool>,
    pub response: Response,
}

#[derive(Default)]
pub(super) struct Tasks {
    pub set: JoinSet<()>,
    pub active: HashMap<Id, Active>,
    pub connections: Vec<(Weak<Session>, Arc<Semaphore>)>,
    pub closed: bool,
}

impl Tasks {
    pub(super) fn joined(
        &self,
        principal: &PrincipalId,
        request: &Request,
    ) -> Result<Option<Response>, BoundaryError> {
        if self.closed {
            return Err(DispatchError::Closed.boundary());
        }
        let active = self.active.values().find(|active| {
            &active.principal == principal && active.request.op_id() == request.op_id()
        });
        match active {
            Some(active) if active.request.as_ref() == request => Ok(Some(active.response.clone())),
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

    pub(super) fn permit(
        &mut self,
        session: &Arc<Session>,
    ) -> Result<OwnedSemaphorePermit, DispatchError> {
        self.reap()?;
        if self.closed {
            return Err(DispatchError::Closed);
        }
        let limit = usize::try_from(Limits::v1().max_in_flight_requests)?;
        self.connections.retain(|(session, permits)| {
            session.strong_count() > 0 || permits.available_permits() < limit
        });
        let permits = if let Some((_, permits)) = self.connections.iter().find(|(candidate, _)| {
            candidate
                .upgrade()
                .is_some_and(|s| Arc::ptr_eq(&s, session))
        }) {
            Arc::clone(permits)
        } else {
            let permits = Arc::new(Semaphore::new(limit));
            self.connections
                .push((Arc::downgrade(session), Arc::clone(&permits)));
            permits
        };
        permits
            .try_acquire_owned()
            .map_err(|_| DispatchError::Backpressure)
    }
}
