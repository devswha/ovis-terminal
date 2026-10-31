//! Typed domain registration; no catalog entry becomes a success-shaped stub.

use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use ovis_terminal_authz::{Permission, Session, TargetResource};
use ovis_terminal_core::ops::Request;
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_protocol::schema::method::{Effect, Targeting};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{Access, Completed, DispatchError, Execution};

/// Implemented domain adapter registered by assembly, using the catalog's closed params.
///
/// `resolve` must consult the owning registry and return its full stable identity and
/// ancestry; it must not mutate or select a focused resource. `run` prepares work only.
/// All effects go through `Execution::commit`, whose returned value cannot be forged.
pub trait Method<D, P>: Debug + Send + Sync + 'static {
    /// Published method name.
    const NAME: &'static str;
    /// The method's existing protocol parameter type, with unknown fields denied.
    type Params: DeserializeOwned + Serialize + JsonSchema + Send + 'static;

    /// Resolves explicit typed parameters to an action and resource scope.
    ///
    /// # Errors
    ///
    /// Returns a typed failure before any domain handler or operation admission runs.
    fn resolve(&self, params: &Self::Params, session: &Session) -> Result<Access, DispatchError>;

    /// Prepares work cooperatively, then commits through the supplied execution.
    fn run(
        &self,
        params: Self::Params,
        execution: Execution<D, P>,
    ) -> impl Future<Output = Result<Completed, DispatchError>> + Send;
}

type RunFuture = Pin<Box<dyn Future<Output = Result<Completed, DispatchError>> + Send>>;
type Run<D, P> = Box<dyn FnOnce(Execution<D, P>) -> RunFuture + Send>;

pub(super) struct Prepared<D, P> {
    pub request: Arc<Request>,
    pub access: Access,
    pub run: Run<D, P>,
}

pub(super) trait Route<D, P>: Send + Sync + Debug {
    fn prepare(
        &self,
        request: &RpcRequest,
        session: &Session,
    ) -> Result<Prepared<D, P>, DispatchError>;
}

#[derive(Debug)]
pub(super) struct Registered<M> {
    pub method: Arc<M>,
    pub effect: Effect,
    pub targeting: Targeting,
}

impl<D: Send + 'static, P: Send + Sync + 'static, M: Method<D, P>> Route<D, P> for Registered<M> {
    fn prepare(
        &self,
        request: &RpcRequest,
        session: &Session,
    ) -> Result<Prepared<D, P>, DispatchError> {
        let params: M::Params = decode_params(request)?;
        let access = self.method.resolve(&params, session)?;
        if (matches!(self.effect, Effect::Mutation)
            && access.action.required() == Permission::Observe)
            || (matches!(self.targeting, Targeting::Terminal)
                && !matches!(access.target.resource(), TargetResource::Terminal { .. }))
        {
            return Err(DispatchError::Registration);
        }
        let mut canonical = serde_json::to_value(&params)?;
        let op_id = match self.effect {
            Effect::Mutation => serde_json::from_value(
                canonical
                    .as_object_mut()
                    .and_then(|object| object.remove("op_id"))
                    .ok_or(DispatchError::Registration)?,
            )?,
            Effect::Query | Effect::Control => OpId::new(uuid::Uuid::now_v7().to_string()),
            _ => return Err(DispatchError::Registration),
        };
        let operation = Request::new(op_id, M::NAME.to_owned(), canonical)?;
        let method = Arc::clone(&self.method);
        Ok(Prepared {
            request: Arc::new(operation),
            access,
            run: Box::new(move |execution| {
                Box::pin(async move { method.run(params, execution).await })
            }),
        })
    }
}
