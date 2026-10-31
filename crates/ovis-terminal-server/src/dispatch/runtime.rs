//! Connection adapter with bounded, dispatcher-owned operation tasks.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use ovis_terminal_authz::Authority;
use ovis_terminal_core::events::{Coordinator, EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_core::registry::{RegistryIdentities, RuntimeIdentity};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_protocol::schema::method::{Effect, MethodSpec};
use ovis_terminal_protocol::schema::methods::all_methods;
use ovis_terminal_protocol::schema::methods::server::OpRefParams;
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{Mutex, Semaphore, watch};
use tokio::time::Instant;

use super::control::{self, Control};
use super::deadline::{receive, wait};
use super::execution::{State, now};
use super::job::Job;
use super::read::{self, ReadRoute, Reader};
use super::registry::{Registered, Route};
use super::tasks::{Active, Tasks};
use super::{DispatchError, Method, Read};
use crate::conn::{Call, RequestHandler};

/// Strict dispatcher sharing the core's actual operation/event coordinator.
///
/// Only `op.get` and `op.cancel` are built in. Domain owners register implemented
/// adapters; missing catalog methods are never advertised as working.
/// Call [`Dispatcher::shutdown`] and await it before closing the coordinator.
pub struct Dispatcher<D, P> {
    state: Arc<State<D, P>>,
    routes: BTreeMap<&'static str, Box<dyn Route<D, P>>>,
    reads: BTreeMap<&'static str, Box<dyn ReadRoute>>,
    tasks: Mutex<Tasks>,
    work: Arc<Semaphore>,
    control: Arc<Semaphore>,
}

impl<D, P> fmt::Debug for Dispatcher<D, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dispatcher")
            .field("methods", &self.routes.keys())
            .field("reads", &self.reads.keys())
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
            reads: BTreeMap::new(),
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
        let spec = self.declared::<M::Params>(M::NAME)?;
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

    /// Registers a query answered from a committed snapshot, outside the operation ledger.
    ///
    /// # Errors
    ///
    /// Rejects unknown, duplicate, non-query, hello and built-in operation-control methods.
    pub fn register_read<R: Read>(&mut self, reader: R) -> Result<(), DispatchError> {
        if !matches!(self.declared::<R::Params>(R::NAME)?.effect, Effect::Query) {
            return Err(DispatchError::Registration);
        }
        self.reads
            .insert(R::NAME, Box::new(Reader(Arc::new(reader))));
        Ok(())
    }

    /// The catalog entry of `name`, if it is unregistered and declares exactly `T` as params.
    fn declared<T: JsonSchema>(&self, name: &str) -> Result<&'static MethodSpec, DispatchError> {
        let spec = all_methods()
            .find(|spec| spec.name == name)
            .ok_or(DispatchError::Registration)?;
        if ["hello", "op.get", "op.cancel"].contains(&name)
            || self.routes.contains_key(name)
            || self.reads.contains_key(name)
        {
            return Err(DispatchError::Registration);
        }
        let mut declared = schemars::SchemaGenerator::default();
        let mut registered = schemars::SchemaGenerator::default();
        if (spec.params)(&mut declared) != registered.subschema_for::<T>()
            || declared.definitions() != registered.definitions()
        {
            return Err(DispatchError::Registration);
        }
        Ok(spec)
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
        if let Some(reader) = self.reads.get(call.request.method()) {
            return read::serve(&self.state, reader.as_ref(), call).await;
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
        let mut tasks = self.tasks.lock().await;
        tasks.reap().map_err(|error| error.boundary())?;
        if let Some(received) = tasks.joined(call.session.principal(), &prepared.request)? {
            drop(tasks);
            return wait(receive(received), op_id, start).await;
        }
        let global = Arc::clone(&self.work)
            .try_acquire_owned()
            .map_err(|_| DispatchError::Backpressure.boundary())?;
        let permit = tasks
            .permit(&call.session)
            .map_err(|error| error.boundary())?;
        let (cancel, cancellation) = watch::channel(false);
        let (reply, received) = watch::channel(None);
        let active = Active {
            principal: call.session.principal().clone(),
            request: Arc::clone(&prepared.request),
            cancel,
            response: received.clone(),
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
            reply.send_replace(Some(result.map_err(|error| error.data().clone())));
            drop((permit, global));
        });
        tasks.active.insert(task.id(), active);
        drop(tasks);
        wait(receive(received), op_id, start).await
    }

    async fn control_call(&self, call: Call, start: Instant) -> Result<Value, BoundaryError> {
        let params: OpRefParams = decode_params(&call.request).map_err(BoundaryError::from)?;
        let cancel = call.request.method() == "op.cancel";
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
                active.principal == *call.session.principal()
                    && active.request.op_id() == &params.op_id
            })
            .map(|active| active.cancel.clone())
            .collect();
        let op_id = params.op_id;
        let control = Control {
            op_id: op_id.clone(),
            cancel,
            signals,
        };
        let (reply, received) = watch::channel(None);
        let state = Arc::clone(&self.state);
        tasks.set.spawn(async move {
            let result = control::run(state, call.session, control).await;
            reply.send_replace(Some(result.map_err(|error| error.data().clone())));
            drop(permit);
        });
        drop(tasks);
        wait(receive(received), op_id, start).await
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
            .chain(self.reads.keys().copied())
            .collect()
    }

    async fn call(&self, call: Call) -> Result<Value, BoundaryError> {
        self.dispatch(call).await
    }
}
