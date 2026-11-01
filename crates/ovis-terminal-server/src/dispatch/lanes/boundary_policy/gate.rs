//! The handler layer: mutation precheck, audit record and error redaction.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::{Authority, AuthzError, Session};
use ovis_terminal_core::registry::RegistryIdentities;
use ovis_terminal_core::telemetry::redaction::redacted;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorData, ErrorKind};
use ovis_terminal_protocol::schema::limits::TransportKind;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use serde_json::Value;

use super::audit::{AuditTrail, Pending};
use super::matrix::required_action;
use crate::authn::refusal;
use crate::conn::{Call, RequestHandler};

/// Serves `inner` behind the boundary policy.
#[derive(Debug)]
pub struct BoundaryPolicy<H> {
    authority: Arc<Authority<RegistryIdentities>>,
    trail: Arc<AuditTrail>,
    inner: H,
}

impl<H> BoundaryPolicy<H> {
    /// Guards `inner` with `authority`, recording every call in `trail`.
    pub fn new(
        (authority, trail): (Arc<Authority<RegistryIdentities>>, Arc<AuditTrail>),
        inner: H,
    ) -> Self {
        Self {
            authority,
            trail,
            inner,
        }
    }

    /// Refuses a mutation whose permission the session's credential confers at no scope.
    fn admit(
        &self,
        (method, session): (&str, &Session),
        at: TimestampMs,
    ) -> Result<(), BoundaryError> {
        let Some(action) = required_action(method) else {
            return Ok(());
        };
        let required = action.required();
        let conferred = self
            .authority
            .confers(session, (required, at))
            .map_err(|error| refusal(&error))?;
        if conferred {
            return Ok(());
        }
        Err(refusal(&AuthzError::Forbidden {
            principal: session.principal().clone(),
            action,
            required,
        }))
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

/// What scrubbing an error's data found.
enum Scrubbed {
    Unchanged,
    Clean(Box<ErrorData>),
    Withheld,
}

fn scrub(data: &ErrorData) -> Scrubbed {
    let Ok(original) = serde_json::to_value(data) else {
        return Scrubbed::Withheld;
    };
    let clean = redacted(&original);
    if clean == original {
        return Scrubbed::Unchanged;
    }
    serde_json::from_value(clean).map_or(Scrubbed::Withheld, |data| Scrubbed::Clean(Box::new(data)))
}

const WITHHELD: &str = "the error detail held secret material and was withheld";

/// The reply error with credentials and secret input removed from its data.
fn withhold_secrets(error: BoundaryError) -> BoundaryError {
    match error {
        BoundaryError::Domain { data } => match scrub(&data) {
            Scrubbed::Unchanged => BoundaryError::Domain { data },
            Scrubbed::Clean(clean) => BoundaryError::Domain { data: clean },
            Scrubbed::Withheld => {
                BoundaryError::domain(data.kind.clone(), data.retryable).with_hint(WITHHELD)
            }
        },
        BoundaryError::InvalidParams { data } => match scrub(&data) {
            Scrubbed::Unchanged => BoundaryError::InvalidParams { data },
            Scrubbed::Clean(clean) => BoundaryError::InvalidParams { data: clean },
            Scrubbed::Withheld => BoundaryError::invalid_params(None),
        },
        other => other,
    }
}

impl<H: RequestHandler> RequestHandler for BoundaryPolicy<H> {
    fn methods(&self) -> Vec<&'static str> {
        self.inner.methods()
    }

    fn transports(&self) -> Vec<TransportKind> {
        self.inner.transports()
    }

    async fn call(&self, call: Call) -> Result<Value, BoundaryError> {
        let at = now()?;
        let pending = Pending::of(at, (&call.request, &call.session));
        let outcome = match self.admit((call.request.method(), &call.session), at) {
            Ok(()) => self.inner.call(call).await.map_err(withhold_secrets),
            Err(refused) => Err(refused),
        };
        self.trail.settle(pending, outcome.as_ref().map(|_| ()));
        outcome
    }
}
