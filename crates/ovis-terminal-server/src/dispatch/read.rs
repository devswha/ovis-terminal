//! Queries answered from one committed snapshot: authorized like every method, but never
//! admitted to the operation ledger, so a read writes no receipt and publishes no event.

use std::fmt::Debug;
use std::sync::Arc;

use ovis_terminal_authz::{Permission, Session};
use ovis_terminal_core::events::{EventError, SnapshotProducer};
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_store::Snapshot;
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::execution::{State, now};
use super::{Access, DispatchError};
use crate::conn::Call;

/// A catalog query served from a committed snapshot without an operation receipt.
///
/// `resolve` names the resource exactly as a [`super::Method`] does; `read` only reads.
pub trait Read: Debug + Send + Sync + 'static {
    /// Published method name; the catalog must declare it a query.
    const NAME: &'static str;
    /// The method's existing protocol parameter type, with unknown fields denied.
    type Params: DeserializeOwned + Serialize + JsonSchema + Send + 'static;

    /// Resolves explicit typed parameters to an observe-level action and resource.
    ///
    /// # Errors
    ///
    /// Returns a typed failure before any snapshot is opened.
    fn resolve(&self, params: &Self::Params, session: &Session) -> Result<Access, DispatchError>;

    /// Answers from `snapshot`, a read-only view of the latest committed revision.
    ///
    /// # Errors
    ///
    /// Returns the typed domain failure to send as the reply.
    fn read(&self, params: Self::Params, snapshot: &Snapshot) -> Result<Value, BoundaryError>;
}

type Answer = Box<dyn FnOnce(&Snapshot) -> Result<Value, BoundaryError> + Send>;

pub(super) trait ReadRoute: Send + Sync + Debug {
    fn prepare(
        &self,
        request: &RpcRequest,
        session: &Session,
    ) -> Result<(Access, Answer), DispatchError>;
}

#[derive(Debug)]
pub(super) struct Reader<R>(pub Arc<R>);

impl<R: Read> ReadRoute for Reader<R> {
    fn prepare(
        &self,
        request: &RpcRequest,
        session: &Session,
    ) -> Result<(Access, Answer), DispatchError> {
        let params: R::Params = decode_params(request)?;
        let access = self.0.resolve(&params, session)?;
        if access.action.required() != Permission::Observe {
            return Err(DispatchError::Registration);
        }
        let reader = Arc::clone(&self.0);
        Ok((
            access,
            Box::new(move |snapshot| reader.read(params, snapshot)),
        ))
    }
}

fn unavailable(error: &EventError) -> BoundaryError {
    BoundaryError::domain(ErrorKind::from(error.kind().to_owned()), false).with_hint(error.hint())
}

/// Authorizes the read, then answers it from a fresh committed snapshot off the executor.
pub(super) async fn serve<D, P>(
    state: &State<D, P>,
    route: &dyn ReadRoute,
    call: Call,
) -> Result<Value, BoundaryError>
where
    D: Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
{
    let (access, answer) = route
        .prepare(&call.request, &call.session)
        .map_err(|error| error.boundary())?;
    let at = now().map_err(|error| error.boundary())?;
    state
        .authority
        .authorize(&call.session, &access.at(at))
        .map_err(|error| DispatchError::from(error).boundary())?;
    let core = Arc::clone(&state.core);
    let answered = tokio::task::spawn_blocking(move || {
        core.read_view(|snapshot| Ok::<_, EventError>(answer(snapshot)))
    })
    .await
    .map_err(|error| DispatchError::from(error).boundary())?;
    answered.map_err(|error| unavailable(&error))?
}
