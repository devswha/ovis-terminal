//! The WebSocket listener: the same handler, authority and runtime identity as the local
//! socket, bound to the loopback interface, owner-account admission for its peers.

use std::sync::Arc;

use ovis_terminal_authz::{Authority, EndpointPolicy, LocalAccount, UnixUid};
use ovis_terminal_core::registry::{RegistryIdentities, RuntimeIdentity};
use ovis_terminal_server::authn::Revalidated;
use ovis_terminal_server::ws::{WsBinding, WsConfig, WsEndpoint, WsServerHandle, serve_websocket};

use super::AssemblyError;
use super::handler::Assembled;

/// Binds `127.0.0.1:<port>` and serves `handler` over WebSocket until shut down.
pub(super) async fn start(
    port: u16,
    (authority, identity): (Arc<Authority<RegistryIdentities>>, RuntimeIdentity),
    handler: &Arc<Revalidated<Assembled>>,
) -> Result<WsServerHandle, AssemblyError> {
    let endpoint = WsEndpoint::bind(WsBinding::loopback(port))
        .await
        .map_err(AssemblyError::start("bind the WebSocket listener"))?;
    let policy = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(
        rustix::process::getuid().as_raw(),
    )));
    let config = WsConfig::new(authority, identity, policy);
    serve_websocket(endpoint, config, Arc::clone(handler))
        .map_err(AssemblyError::start("serve the WebSocket listener"))
}
