//! One redacted audit record per call, readable only by an administrator.

use std::sync::{Mutex, PoisonError};

use ovis_terminal_authz::{
    AccessRequest, Action, Authority, ResolvedTarget, Session, TargetResource,
};
use ovis_terminal_core::registry::{RegistryIdentities, RuntimeIdentity};
use ovis_terminal_core::telemetry::redaction::{AuditLog, RetentionPolicy, redacted};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use serde_json::{Map, Value, json};

use crate::authn::refusal;

/// Redacted parameters larger than this are recorded by size only.
const PARAMS_BYTES: usize = 4096;

/// Parameters that identify what a call acted on, kept beside the redacted parameters.
const IDENTITIES: [&str; 5] = ["op_id", "target", "prompt_id", "machine", "name"];

/// The audit trail of one runtime: what was asked, by whom, of what, and how it ended.
#[derive(Debug)]
pub struct AuditTrail {
    identity: RuntimeIdentity,
    log: Mutex<AuditLog>,
}

/// The record of a call taken before it runs, so its payload is never held twice.
pub(super) struct Pending {
    at: TimestampMs,
    record: Map<String, Value>,
}

impl Pending {
    /// Describes `request` by `session`, received at `at`.
    pub(super) fn of(at: TimestampMs, (request, session): (&RpcRequest, &Session)) -> Self {
        let params = request.params();
        let mut record = Map::new();
        record.insert("at_ms".to_owned(), Value::String(at.get().to_string()));
        record.insert("request".to_owned(), json!(request.id()));
        record.insert("method".to_owned(), json!(request.method()));
        record.insert("principal".to_owned(), json!(session.principal().as_str()));
        for key in IDENTITIES {
            if let Some(value) = params.get(key) {
                record.insert(key.to_owned(), redacted(value));
            }
        }
        record.insert("params".to_owned(), bounded(params));
        Self { at, record }
    }
}

impl AuditTrail {
    /// An empty trail of the runtime `identity` under `policy`.
    #[must_use]
    pub const fn new(identity: RuntimeIdentity, policy: RetentionPolicy) -> Self {
        Self {
            identity,
            log: Mutex::new(AuditLog::new(policy)),
        }
    }

    /// Retains the record of a settled call with its outcome.
    pub(super) fn settle(&self, pending: Pending, outcome: Result<(), &BoundaryError>) {
        let Pending { at, mut record } = pending;
        let (settled, kind) = match outcome {
            Ok(()) => ("ok", Value::Null),
            Err(error) => ("refused", json!(error.kind())),
        };
        record.insert("outcome".to_owned(), json!(settled));
        record.insert("error_kind".to_owned(), kind);
        let (machine, runtime) = (self.identity.machine_id(), self.identity.runtime_id());
        record.insert("machine_id".to_owned(), json!(machine.to_string()));
        record.insert("runtime_id".to_owned(), json!(runtime.to_string()));
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .append((at, Value::Object(record)));
    }

    /// The retained records, for a session the authority lets administer the machine.
    ///
    /// # Errors
    ///
    /// The authority's refusal, such as `forbidden` for a non-administrator.
    pub fn read(
        &self,
        authority: &Authority<RegistryIdentities>,
        (session, at): (&Session, TimestampMs),
    ) -> Result<Value, BoundaryError> {
        let target = ResolvedTarget::new(self.identity.machine_id(), TargetResource::Machine);
        authority
            .authorize(session, &AccessRequest::new(Action::Administer, target, at))
            .map_err(|error| refusal(&error))?;
        let mut log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
        let records = log.records(at);
        Ok(json!({"records": records, "evicted": log.evicted().to_string()}))
    }
}

/// The redacted parameters, or only their size when they are too large to keep.
fn bounded(params: &Value) -> Value {
    let copy = redacted(params);
    let size = serde_json::to_vec(&copy).map_or(usize::MAX, |bytes| bytes.len());
    if size <= PARAMS_BYTES {
        copy
    } else {
        json!({"omitted_bytes": size.to_string()})
    }
}
