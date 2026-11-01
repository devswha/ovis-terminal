//! Running a plan. Only `connect`, `server status`, `terminal get` without a target and
//! method calls reach the endpoint, and none of them can start a server.

use std::time::Duration;

use ovis_terminal_protocol::errors::TargetCandidate;
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::common::NoParams;
use ovis_terminal_protocol::schema::ids::{MachineId, TerminalId};
use ovis_terminal_protocol::schema::methods::server::ServerStatusResult;
use ovis_terminal_protocol::schema::methods::terminal::TerminalListParams;
use ovis_terminal_server::listen::{EndpointAddress, EndpointDescriptor, Platform};
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

fn address(context: &Context) -> Result<EndpointAddress, CliError> {
    EndpointDescriptor::resolve(context.endpoint(), Platform::CURRENT)
        .map(|descriptor| descriptor.address().clone())
        .map_err(CliError::Endpoint)
}

async fn bounded<T>(work: impl Future<Output = Result<T, Failure>>) -> Result<T, Failure> {
    tokio::time::timeout(DEADLINE, work).await.map_err(|_| {
        Failure::from(CliError::Timeout {
            deadline_ms: DEADLINE_MS,
        })
    })?
}

/// Sends one planned request and returns its result untouched.
pub(super) async fn call(context: &Context, request: &RpcRequest) -> Result<Value, Failure> {
    let address = address(context)?;
    bounded(async {
        let mut session = Session::open(&address, context.credential()).await?;
        session.call(request).await
    })
    .await
}

fn idle_status(state: &str) -> Result<Value, Failure> {
    published_status(json!({
        "state": state,
        "runtime_lifetime": null,
        "server_boot_id": null,
        "uptime_ms": null,
        "holder_count": null,
        "live_terminals": null,
        "census": "not_reported",
    }))
}

/// Decodes a status record as the published result type and encodes it back.
pub(super) fn published_status(record: Value) -> Result<Value, Failure> {
    let status: ServerStatusResult = serde_json::from_value(record).map_err(CliError::Result)?;
    Ok(serde_json::to_value(status).map_err(CliError::Encode)?)
}

/// Observes the endpoint without creating, binding or starting anything: a missing or
/// refused endpoint is reported locally, one that accepts but never answers hello is
/// `unreachable`, and only a server that completes hello is asked `server.status`.
pub(super) async fn status(context: &Context) -> Result<Value, Failure> {
    let address = address(context)?;
    let opened = tokio::time::timeout(DEADLINE, Session::open(&address, context.credential()));
    let mut session = match opened.await {
        Ok(Ok(session)) => session,
        Ok(Err(Failure::Cli(CliError::NotRunning { state, .. }))) => return idle_status(state),
        Ok(Err(Failure::Cli(CliError::Closed))) | Err(_) => return idle_status("unreachable"),
        Ok(Err(other)) => return Err(other),
    };
    let request = typed::<NoParams>("server.status", &json!({}))?;
    published_status(bounded(session.call(&request)).await?)
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
    let address = match address(context) {
        Ok(address) => address,
        Err(error) => return error.into(),
    };
    let listed = bounded(async {
        let mut session = Session::open(&address, context.credential()).await?;
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

/// Attaches until detach; a server refusal stays the server's typed error.
#[cfg(unix)]
pub(super) async fn attach(
    context: &Context,
    plan: &crate::attach::AttachPlan,
) -> Result<super::Report, Failure> {
    let address = address(context)?;
    let bearer = context.credential().map(super::context::Credential::bearer);
    match crate::attach::attach(plan, (&address, bearer)).await {
        Ok(record) => Ok(super::Report::detached(&record)),
        Err(crate::attach::AttachError::Server(error)) => Err(Failure::Server(error)),
        Err(error) => Err(CliError::Attach(error).into()),
    }
}

/// Relays protocol bytes between stdio and the endpoint.
pub(super) async fn connect(context: &Context) -> Result<(), CliError> {
    bridge(&address(context)?).await
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
            "holder_count": "1",
            "live_terminals": "0",
            "census": "reported",
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
