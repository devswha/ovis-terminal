//! The request handler wrapper that revalidates the session behind every call.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::{Authority, AuthzError};
use ovis_terminal_core::registry::RegistryIdentities;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use serde_json::Value;

use crate::conn::{Call, RequestHandler};

/// Serves `inner` only to sessions whose credential is still valid at the request.
#[derive(Debug)]
pub struct Revalidated<H> {
    authority: Arc<Authority<RegistryIdentities>>,
    inner: H,
}

impl<H> Revalidated<H> {
    /// Guards `inner` with `authority`, the authority that authenticated the sessions.
    pub const fn new(authority: Arc<Authority<RegistryIdentities>>, inner: H) -> Self {
        Self { authority, inner }
    }

    /// The guarded handler.
    pub const fn inner(&self) -> &H {
        &self.inner
    }
}

/// The wire refusal of an authority failure, naming the principal's current epoch when
/// the failure is about its authority moving.
pub fn refusal(error: &AuthzError) -> BoundaryError {
    let refused = BoundaryError::domain(ErrorKind::from(error.kind().to_owned()), false)
        .with_hint(error.hint());
    match error.principal_epoch() {
        Some(epoch) => refused.with_principal_epoch(epoch),
        None => refused,
    }
}

fn now() -> Result<TimestampMs, BoundaryError> {
    let clock = || {
        BoundaryError::domain(ErrorKind::from("internal".to_owned()), false)
            .with_hint("correct the runtime clock")
    };
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| clock())?;
    u64::try_from(elapsed.as_millis())
        .map(TimestampMs::new)
        .map_err(|_| clock())
}

impl<H: RequestHandler> RequestHandler for Revalidated<H> {
    fn methods(&self) -> Vec<&'static str> {
        self.inner.methods()
    }

    async fn call(&self, call: Call) -> Result<Value, BoundaryError> {
        self.authority
            .validate(&call.session, now()?)
            .map_err(|error| refusal(&error))?;
        self.inner.call(call).await
    }
}
