//! The seam between an established connection and method dispatch.

use std::future::Future;
use std::sync::Arc;

use ovis_terminal_authz::Session;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::limits::TransportKind;
use serde_json::Value;

use super::Notifier;

/// One request accepted on an authenticated connection.
#[derive(Debug)]
#[non_exhaustive]
pub struct Call {
    /// The request; its id is answered exactly once by the connection.
    pub request: RpcRequest,
    /// The principal that completed hello on this connection.
    pub session: Arc<Session>,
    /// Queues notifications, such as subscription events, on the same connection.
    pub notifier: Notifier,
}

/// Serves the methods of established connections; hello never reaches it.
pub trait RequestHandler: Send + Sync + 'static {
    /// Catalog names of the methods this handler implements; hello advertises exactly these.
    fn methods(&self) -> Vec<&'static str>;

    /// Transports the server serving this handler listens on; hello advertises them.
    fn transports(&self) -> Vec<TransportKind> {
        vec![TransportKind::Unix]
    }

    /// Settles one call with a result or a typed error.
    fn call(&self, call: Call) -> impl Future<Output = Result<Value, BoundaryError>> + Send;
}
