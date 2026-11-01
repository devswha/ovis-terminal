//! Remote dispatch (plan todo 52): a request whose target names another machine goes to
//! that machine's actor and never to a local resource.
//!
//! A request is remote when `params.target.address` is qualified by another machine
//! (`<machine>/<terminal>`) or `params.machine` names one. The machine is resolved once by
//! identity, name or alias and the request is forwarded with the qualifier removed. A
//! machine without a line refuses at once as `machine_unreachable` with the last remote
//! sequence it mirrored; nothing runs locally instead. A mutation whose line is lost after
//! it was sent may have run on the machine: it settles as `outcome_unknown` with its
//! operation id, never as a failure that invites a blind retry.
//!
//! The caller is authorized here against this runtime's machine scope (reads need observe,
//! input needs interact, a launch needs launch, other mutations administer); the machine
//! then authorizes the forwarded request against the credential its line presented.

mod attach;
mod refusal;
mod route;

use std::sync::Arc;
use std::time::Duration;

pub use attach::{MachineAddAttached, ReconnectError, reconnect};
use ovis_terminal_authz::{Action, Authority, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::registry::{MachineId as LocalMachine, RegistryIdentities};
use ovis_terminal_protocol::errors::{BoundaryError, OpId};
use ovis_terminal_protocol::schema::methods::machine::MachineState;
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use ovis_terminal_remote::actors::{ActorError, Actors, Line, Request};
use serde_json::Value;

use self::route::{Remote, Route};
use super::execution::now;
use super::{Access, DispatchError};
use crate::conn::{Call, RequestHandler};

/// The machine actors remote requests travel through and the bound on each call.
#[derive(Debug)]
pub struct RemoteRoutes<L> {
    actors: Arc<Actors<L>>,
    authority: Arc<Authority<RegistryIdentities>>,
    local: LocalMachine,
    deadline: Duration,
}

impl<L: Line> RemoteRoutes<L> {
    /// Routes of the runtime on `local` through `actors`, each call bounded by `deadline`
    /// and authorized by `authority`.
    #[must_use]
    pub fn new(
        (actors, authority): (Arc<Actors<L>>, Arc<Authority<RegistryIdentities>>),
        local: LocalMachine,
        deadline: Duration,
    ) -> Self {
        Self {
            actors,
            authority,
            local,
            deadline,
        }
    }

    fn authorize(&self, session: &Session, action: Action) -> Result<(), BoundaryError> {
        let target = ResolvedTarget::new(self.local, TargetResource::Machine);
        let at = now().map_err(|error| error.boundary())?;
        self.authority
            .authorize(session, &Access::new(action, target).at(at))
            .map(drop)
            .map_err(|error| DispatchError::from(error).boundary())
    }

    async fn forward(&self, call: &Call, remote: Remote) -> Result<Value, BoundaryError> {
        let Remote { machine, params } = remote;
        let method = call.request.method();
        let op = params
            .get("op_id")
            .and_then(Value::as_str)
            .map(|op| OpId::new(op.to_owned()));
        self.authorize(&call.session, route::action(method, op.is_some()))?;
        let health = self
            .actors
            .health(&machine)
            .map_err(|error| refusal::actor(&error))?;
        if health.state != OpenEnum::Known(MachineState::Connected) {
            return Err(refusal::unreachable(&machine, health.last_seen_seq));
        }
        let sent = Request::new(method, params, self.deadline);
        match self.actors.call(&machine, sent).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(error)) => Err(refusal::answered(&error)),
            Err(ActorError::Unreachable { last_seen_seq, .. }) => Err(op.map_or_else(
                || refusal::unreachable(&machine, last_seen_seq),
                |op| refusal::outcome_unknown(&machine, op, last_seen_seq),
            )),
            Err(error) => Err(refusal::with_op(refusal::actor(&error), op)),
        }
    }
}

/// Serves remote requests through [`RemoteRoutes`] and every other request locally.
#[derive(Debug)]
pub struct RemoteDispatch<L, H> {
    routes: RemoteRoutes<L>,
    local: Arc<H>,
}

impl<L: Line, H: RequestHandler> RemoteDispatch<L, H> {
    /// `local` behind `routes`.
    #[must_use]
    pub const fn new(routes: RemoteRoutes<L>, local: Arc<H>) -> Self {
        Self { routes, local }
    }
}

impl<L: Line, H: RequestHandler> RequestHandler for RemoteDispatch<L, H> {
    fn methods(&self) -> Vec<&'static str> {
        self.local.methods()
    }

    async fn call(&self, call: Call) -> Result<Value, BoundaryError> {
        match route::of(call.request.params(), &self.routes)? {
            Route::Local => self.local.call(call).await,
            Route::Remote(remote) => self.routes.forward(&call, remote).await,
        }
    }
}

#[cfg(all(test, unix))]
mod qa;
