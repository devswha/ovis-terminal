//! The live identity a server reports in `hello`.

use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::Transport;
use ovis_terminal_core::registry::RuntimeIdentity;
use ovis_terminal_protocol::schema::limits::{Limits, TransportKind};
use ovis_terminal_protocol::schema::methods::server::HelloResult;
use ovis_terminal_protocol::schema::scalar::{OpenEnum, TimestampMs};
use ovis_terminal_protocol::schema::{PROTOCOL_GENERATION, advertised_methods};
use serde_json::json;

use super::HelloError;
use crate::listen::EndpointError;

/// Machine, runtime-lifetime and boot identity plus what this build serves.
#[derive(Debug, Clone)]
pub struct ServerIdentity {
    runtime: RuntimeIdentity,
    template: HelloResult,
}

impl ServerIdentity {
    /// Describes a server of `runtime` whose handler implements `methods`.
    ///
    /// # Errors
    ///
    /// [`EndpointError::Methods`] for a method outside the published catalog and
    /// [`EndpointError::Identity`] when the identity does not form a hello result.
    pub fn new(runtime: RuntimeIdentity, methods: &[&str]) -> Result<Self, EndpointError> {
        let methods =
            advertised_methods(methods.iter().copied()).map_err(EndpointError::Methods)?;
        let template = serde_json::from_value(json!({
            "machine_id": runtime.machine_id().to_string(),
            "runtime_lifetime": runtime.runtime_id().to_string(),
            "server_boot_id": runtime.server_boot_id().to_string(),
            "generation": PROTOCOL_GENERATION,
            "version": env!("CARGO_PKG_VERSION"),
            "methods": methods,
            "capabilities": {
                "transports": [TransportKind::Unix],
                "stream_modes": [],
                "framed": true,
            },
            "limits": Limits::v1(),
            "server_time_ms": TimestampMs::new(0),
            "endpoint_kind": TransportKind::Unix,
        }))
        .map_err(EndpointError::Identity)?;
        Ok(Self { runtime, template })
    }

    /// The runtime identity this server reports.
    #[must_use]
    pub const fn runtime(&self) -> RuntimeIdentity {
        self.runtime
    }

    pub(super) const fn limits(&self) -> &Limits {
        &self.template.limits
    }

    pub(super) fn hello_result(&self, transport: Transport, at: TimestampMs) -> HelloResult {
        let mut result = self.template.clone();
        result.server_time_ms = at;
        result.endpoint_kind = match transport {
            Transport::UnixSocket => OpenEnum::Known(TransportKind::Unix),
            Transport::NamedPipe => OpenEnum::Known(TransportKind::Pipe),
            Transport::WebSocket => OpenEnum::Known(TransportKind::Websocket),
            Transport::Ssh => OpenEnum::Known(TransportKind::Stdio),
            other => OpenEnum::Unknown(other.as_str().to_owned()),
        };
        result
    }
}

pub(super) fn now_ms() -> Result<TimestampMs, HelloError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| HelloError::Clock)?;
    let millis = u64::try_from(elapsed.as_millis()).map_err(|_| HelloError::Clock)?;
    Ok(TimestampMs::new(millis))
}
