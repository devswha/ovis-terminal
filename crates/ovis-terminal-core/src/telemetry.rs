//! Structured tracing context, bounded counters, and serialization redaction.
//!
//! Records go only to an injected sink. This module never installs a global
//! logger or writes diagnostics to protocol stdout.

mod context;
mod counters;
mod serialize;
mod sink;

pub use context::{
    FieldValue, MachineId, OperationId, Outcome, PrincipalId, RequestId, ResourceId, RuntimeId,
    TraceContext,
};
pub use counters::{Counter, Counters};
pub use sink::{TelemetryError, TelemetrySink, emit, log_handled_error};
