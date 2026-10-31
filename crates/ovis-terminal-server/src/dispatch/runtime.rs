//! Connection adapter with bounded, dispatcher-owned operation tasks.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use ovis_terminal_authz::{Action, Authority, ResolvedTarget, TargetResource};
use ovis_terminal_core::events::{Coordinator, EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_core::registry::{RegistryIdentities, RuntimeIdentity};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_protocol::schema::methods::all_methods;
use ovis_terminal_protocol::schema::methods::server::OpRefParams;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{Mutex, Semaphore, oneshot, watch};
use tokio::time::Instant;

use super::deadline::wait;
use super::execution::{State, now};
use super::job::{Control, Job};
use super::registry::{Registered, Route};
use super::tasks::{Active, Tasks};
use super::{Access, DispatchError, Method};
use crate::conn::{Call, RequestHandler};

/// Strict dispatcher sharing the core's actual operation/event coordinator.
///
/// Only `op.get` and `op.cancel` are built in. Domain owners register implemented
/// adapters; missing catalog methods are never advertised as working.
/// Call [`Dispatcher::shutdown`] and await it before closing the coordinator.
pub struct Dispatcher<D, P> {
    state: Arc<State<D, P>>,
    routes: BTreeMap<&'static str, Box<dyn Route<D, P>>>,
    tasks: Mutex<Tasks>,
    work: Arc<Semaphore>,
    control: Arc<Semaphore>,
}

impl<D, P> fmt::Debug for Dispatcher<D, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dispatcher")
            .field("methods", &self.routes.keys())
            .finish_non_exhaustive()
    }
}

impl<D, P> Dispatcher<D, P>
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    /// Builds dispatch over the same authority, runtime and coordinator as the domains.
    pub fn new(
        authority: Arc<Authority<RegistryIdentities>>,
        identity: RuntimeIdentity,
        core: Arc<Coordinator<D, P>>,
    ) -> Self {
        Self {
            state: Arc::new(State {
                authority,
                identity,
                core,
            }),
            routes: BTreeMap::new(),
            tasks: Mutex::new(Tasks::default()),
            work: Arc::new(Semaphore::new(128)),
            control: Arc::new(Semaphore::new(16)),
        }
    }

    /// Registers an implemented module before sharing the dispatcher.
    ///
    /// # Errors
    ///
    /// Rejects unknown, duplicate, hello and built-in operation-control methods.
    pub fn register<M: Method<D, P>>(&mut self, method: M) -> Result<(), DispatchError> {
        let spec = all_methods()
            .find(|spec| spec.name == M::NAME)
            .ok_or(DispatchError::Registration)?;
        if ["hello", "op.get", "op.cancel"].contains(&M::NAME) || self.routes.contains_key(M::NAME)
        {
            return Err(DispatchError::Registration);
        }
        let mut declared = schemars::SchemaGenerator::default();
        let mut registered = schemars::SchemaGenerator::default();
        if (spec.params)(&mut declared) != registered.subschema_for::<M::Params>()
            || declared.definitions() != registered.definitions()
        {
            return Err(DispatchError::Registration);
        }
        self.routes.insert(
            M::NAME,
            Box::new(Registered {
                method: Arc::new(method),
                effect: spec.effect,
                targeting: spec.targeting,
            }),
        );
        Ok(())
    }

    /// Cooperatively cancels outstanding work and joins every task, including blocking effects.
    ///
    /// # Errors
    ///
    /// Reports an owned task panic. Effects already started are joined, never rolled back.
    pub async fn shutdown(&self) -> Result<(), DispatchError> {
        let mut tasks = self.tasks.lock().await;
        tasks.closed = true;
        for active in tasks.active.values() {
            active.cancel.send_replace(true);
        }
        let running = tasks.take_running();
        drop(tasks);
        running.join().await
    }

    /// Joins outstanding work without cancelling it; callers must stop submitting first.
    ///
    /// # Errors
    ///
    /// Reports an owned task panic.
    pub async fn drain(&self) -> Result<(), DispatchError> {
        let mut tasks = self.tasks.lock().await;
        let running = tasks.take_running();
        drop(tasks);
        running.join().await
    }

    async fn dispatch(&self, call: Call) -> Result<Value, BoundaryError> {
        let start = Instant::now();
        let is_control = matches!(call.request.method(), "op.get" | "op.cancel");
        if is_control {
            return self.control_call(call, start).await;
        }
        let route = self
            .routes
            .get(call.request.method())
            .ok_or_else(BoundaryError::method_not_found)?;
        let prepared = route
            .prepare(&call.request, &call.session)
            .map_err(|error| error.boundary())?;
        let at = now().map_err(|error| error.boundary())?;
        let decision = self
            .state
            .authority
            .authorize(&call.session, &prepared.access.at(at))
            .map_err(|error| DispatchError::from(error).boundary())?;
        let op_id = prepared.request.op_id().clone();
        let global = Arc::clone(&self.work)
            .try_acquire_owned()
            .map_err(|_| DispatchError::Backpressure.boundary())?;
        let mut tasks = self.tasks.lock().await;
        let permit = tasks
            .permit(&call.session)
            .map_err(|error| error.boundary())?;
        let (cancel, cancellation) = watch::channel(false);
        let (reply, received) = oneshot::channel();
        let active = Active {
            principal: call.session.principal().clone(),
            op_id: op_id.clone(),
            cancel,
        };
        let job = Job {
            state: Arc::clone(&self.state),
            session: call.session,
            decision,
            notifier: call.notifier,
        };
        let task = tasks.set.spawn(async move {
            let result = job.run(prepared, cancellation).await;
            // A disappeared RPC waiter does not cancel or discard the durable operation.
            match reply.send(result) {
                Ok(()) | Err(_) => {}
            }
            drop((permit, global));
        });
        tasks.active.insert(task.id(), active);
        drop(tasks);
        wait(received, op_id, start).await
    }

    async fn control_call(&self, call: Call, start: Instant) -> Result<Value, BoundaryError> {
        let params: OpRefParams = decode_params(&call.request).map_err(BoundaryError::from)?;
        let cancel = call.request.method() == "op.cancel";
        let access = Access::new(
            if cancel {
                Action::CancelOperation
            } else {
                Action::InspectOperation
            },
            ResolvedTarget::new(
                self.state.identity.machine_id(),
                TargetResource::Operation {
                    principal: call.session.principal().clone(),
                    op_id: params.op_id.clone(),
                },
            ),
        );
        let at = now().map_err(|error| error.boundary())?;
        let decision = self
            .state
            .authority
            .authorize(&call.session, &access.at(at))
            .map_err(|error| DispatchError::from(error).boundary())?;
        let permit = Arc::clone(&self.control)
            .try_acquire_owned()
            .map_err(|_| DispatchError::Backpressure.boundary())?;
        let mut tasks = self.tasks.lock().await;
        tasks.reap().map_err(|error| error.boundary())?;
        if tasks.closed {
            return Err(DispatchError::Closed.boundary());
        }
        let signals = tasks
            .active
            .values()
            .filter(|active| {
                active.principal == *call.session.principal() && active.op_id == params.op_id
            })
            .map(|active| active.cancel.clone())
            .collect();
        let op_id = params.op_id;
        let control = Control {
            op_id: op_id.clone(),
            access,
            cancel,
        };
        let (reply, received) = oneshot::channel();
        let job = Job {
            state: Arc::clone(&self.state),
            session: call.session,
            decision,
            notifier: call.notifier,
        };
        tasks.set.spawn(async move {
            let result = job.control(control, signals).await;
            match reply.send(result) {
                Ok(()) | Err(_) => {}
            }
            drop(permit);
        });
        drop(tasks);
        wait(received, op_id, start).await
    }
}

impl<D, P> RequestHandler for Dispatcher<D, P>
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    fn methods(&self) -> Vec<&'static str> {
        ["op.get", "op.cancel"]
            .into_iter()
            .chain(self.routes.keys().copied())
            .collect()
    }

    async fn call(&self, call: Call) -> Result<Value, BoundaryError> {
        self.dispatch(call).await
    }
}
