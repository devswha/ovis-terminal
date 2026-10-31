//! Running a plan. Only `connect`, `server status`, `terminal get` without a target and
//! method calls reach the endpoint, and none of them can start a server.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ovis_terminal_protocol::errors::TargetCandidate;
use ovis_terminal_protocol::schema::ids::{MachineId, TerminalId};
use ovis_terminal_protocol::schema::methods::server::{ServerState, ServerStatusResult};
use ovis_terminal_protocol::schema::methods::terminal::TerminalListParams;
use ovis_terminal_server::listen::{EndpointDescriptor, EndpointStatus, Platform, Probe};
use serde::Deserialize;
use serde_json::{Value, json};

use super::context::Context;
use super::error::CliError;
use super::plan::typed;
use super::report::Failure;
use super::transport::{Session, bridge};

/// Bound on one exchange in milliseconds, the protocol's ordinary deadline.
const DEADLINE_MS: u64 = 10_000;
const DEADLINE: Duration = Duration::from_millis(DEADLINE_MS);

fn socket(context: &Context) -> Result<PathBuf, CliError> {
    let descriptor = EndpointDescriptor::resolve(context.endpoint(), Platform::CURRENT)
        .map_err(CliError::Endpoint)?;
    descriptor
        .unix_path()
        .map(Path::to_owned)
        .ok_or(CliError::UnsupportedTransport)
}

async fn bounded<T>(work: impl Future<Output = Result<T, Failure>>) -> Result<T, Failure> {
    tokio::time::timeout(DEADLINE, work).await.map_err(|_| {
        Failure::from(CliError::Timeout {
            deadline_ms: DEADLINE_MS,
        })
    })?
}

/// Sends one planned request and returns its result untouched.
pub(super) async fn call(
    context: &Context,
    request: &ovis_terminal_protocol::rpc::RpcRequest,
) -> Result<Value, Failure> {
    let path = socket(context)?;
    bounded(async {
        let mut session = Session::open(&path, context.credential()).await?;
        session.call(request).await
    })
    .await
}

fn idle_status(state: ServerState) -> Result<Value, Failure> {
    let record = json!({
        "state": state,
        "runtime_lifetime": null,
        "server_boot_id": null,
        "uptime_ms": null,
        "holder_count": 0,
        "live_terminals": 0,
    });
    published_status(record)
}

/// Decodes a status record as the published result type and encodes it back.
pub(super) fn published_status(record: Value) -> Result<Value, Failure> {
    let status: ServerStatusResult = serde_json::from_value(record).map_err(CliError::Result)?;
    Ok(serde_json::to_value(status).map_err(CliError::Encode)?)
}

/// Observes the endpoint without creating, binding or starting anything.
pub(super) async fn status(context: &Context) -> Result<Value, Failure> {
    let path = socket(context)?;
    let bearer = context
        .credential()
        .map_or("", |credential| credential.bearer());
    let observed =
        Probe::new(bearer, DEADLINE)
            .observe(&path)
            .await
            .map_err(|error| match error {
                ovis_terminal_server::listen::EndpointError::HelloRejected { error } => {
                    Failure::Server(error)
                }
                other => CliError::Endpoint(other).into(),
            })?;
    match observed {
        EndpointStatus::Running(_) => {
            let request = typed::<ovis_terminal_protocol::schema::common::NoParams>(
                "server.status",
                &json!({}),
            )?;
            published_status(call(context, &request).await?)
        }
        other => idle_status(other.state()),
    }
}

#[derive(Deserialize)]
struct Listed {
    terminal_id: TerminalId,
    machine_id: MachineId,
}

#[derive(Deserialize)]
struct Listing {
    terminals: Vec<Listed>,
}

/// Refuses a targetless read with the live candidates; it never picks one.
pub(super) async fn needs_target(context: &Context) -> Failure {
    let path = match socket(context) {
        Ok(path) => path,
        Err(error) => return error.into(),
    };
    let listed = bounded(async {
        let mut session = Session::open(&path, context.credential()).await?;
        let request = typed::<TerminalListParams>("terminal.list", &json!({}))?;
        let listing: Listing =
            serde_json::from_value(session.call(&request).await?).map_err(CliError::Result)?;
        let lifetime = session.hello().runtime_lifetime.clone();
        Ok(listing
            .terminals
            .into_iter()
            .map(|row| TargetCandidate::new(row.machine_id, lifetime.clone(), row.terminal_id))
            .collect())
    })
    .await;
    match listed {
        Ok(candidates) => CliError::NeedsTarget {
            candidates,
            listed: true,
        }
        .into(),
        Err(Failure::Cli(CliError::NotRunning { .. })) => CliError::NeedsTarget {
            candidates: Vec::new(),
            listed: false,
        }
        .into(),
        Err(other) => other,
    }
}

/// Relays protocol bytes between stdio and the endpoint.
pub(super) async fn connect(context: &Context) -> Result<(), CliError> {
    bridge(&socket(context)?).await
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::json;

    use super::published_status;
    use crate::invocation::report::Failure;

    fn record(uptime: &serde_json::Value) -> serde_json::Value {
        json!({
            "state": "running",
            "runtime_lifetime": "lifetime-7",
            "server_boot_id": "boot-1",
            "uptime_ms": uptime,
            "holder_count": 1,
            "live_terminals": 0,
        })
    }

    proptest! {
        #[test]
        fn uptime_is_relayed_as_the_exact_decimal_string(uptime in any::<u64>()) {
            let relayed = published_status(record(&json!(uptime.to_string())));
            prop_assert_eq!(relayed.ok().map(|value| value.get("uptime_ms").cloned()), Some(Some(json!(uptime.to_string()))));
        }
    }

    #[test]
    fn u64_max_round_trips_and_a_numeric_uptime_is_an_invalid_response() {
        let relayed = published_status(record(&json!(u64::MAX.to_string())));
        assert_eq!(
            relayed.ok().map(|value| value.get("uptime_ms").cloned()),
            Some(Some(json!("18446744073709551615")))
        );
        let numeric = published_status(record(&json!(u64::MAX)));
        assert!(matches!(numeric, Err(Failure::Cli(error)) if error.kind() == "invalid_response"));
    }
}
