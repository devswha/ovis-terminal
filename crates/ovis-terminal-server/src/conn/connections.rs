//! Serving authenticated connections over any byte stream a listener accepted.

use std::future::Future;
use std::sync::Arc;

use ovis_terminal_authz::{Authority, AuthzError, PeerAdmission};
use ovis_terminal_core::registry::{RegistryIdentities, RuntimeIdentity};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;

use super::core::ConnectionCore;
use super::driver::{DriverContext, drive};
use super::hello::Shared;
use super::{ConnectionError, RequestHandler, ServerIdentity};
use crate::listen::EndpointError;

/// Everything the connections of one server share, and the signal that closes them.
#[derive(Debug)]
pub struct Connections<H> {
    shared: Arc<Shared>,
    handler: Arc<H>,
    max_in_flight: usize,
    signal: watch::Sender<bool>,
}

impl<H: RequestHandler> Connections<H> {
    /// Connections of `runtime` authenticating against `authority` and served by `handler`.
    ///
    /// # Errors
    ///
    /// [`EndpointError::Methods`] or [`EndpointError::Identity`] when the hello result cannot
    /// be formed, and [`EndpointError::Unrepresentable`] for a limit this platform cannot hold.
    pub fn new(
        authority: Arc<Authority<RegistryIdentities>>,
        runtime: RuntimeIdentity,
        handler: Arc<H>,
    ) -> Result<Self, EndpointError> {
        let identity = ServerIdentity::new(runtime, &handler.methods())?;
        let max_in_flight =
            usize::try_from(identity.limits().max_in_flight_requests).map_err(|_| {
                EndpointError::Unrepresentable {
                    limit: "max_in_flight_requests",
                }
            })?;
        let (signal, _) = watch::channel(false);
        Ok(Self {
            shared: Arc::new(Shared {
                authority,
                identity,
            }),
            handler,
            max_in_flight,
            signal,
        })
    }

    /// Serves one accepted stream whose endpoint admission produced `admission`.
    ///
    /// The returned future ends when the peer closes, a failure closes the connection or
    /// [`Connections::shutdown`] is called; dropping it cancels every request it started.
    pub fn serve<S>(
        &self,
        stream: S,
        admission: Result<PeerAdmission, AuthzError>,
    ) -> impl Future<Output = Result<(), ConnectionError>> + Send + 'static
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let core = ConnectionCore::new(Arc::clone(&self.shared), admission);
        let context = DriverContext {
            handler: Arc::clone(&self.handler),
            shutdown: self.signal.subscribe(),
            max_in_flight: self.max_in_flight,
        };
        drive(stream, core, context)
    }

    /// Closes every served connection with `$/closing` and cancels its requests.
    pub fn shutdown(&self) {
        self.signal.send_replace(true);
    }
}
