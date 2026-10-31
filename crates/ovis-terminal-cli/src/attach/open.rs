//! `attach.open` with its typed outcomes, and the geometry lease the attach was granted.

use ovis_terminal_protocol::schema::methods::attach::{AttachOpenResult, AttachOutcome};
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use serde_json::{Value, json};

use super::client::Client;
use super::{AttachError, AttachPlan, tty};

fn op_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

fn unreadable(detail: &'static str) -> impl FnOnce(serde_json::Error) -> AttachError {
    move |_| AttachError::Protocol { detail }
}

/// Opens the attach and turns a typed refusal into its error; a held lease is never
/// taken over and its observe stream is closed again.
pub(super) async fn open(
    client: &Client,
    plan: &AttachPlan,
) -> Result<AttachOpenResult, AttachError> {
    let geometry = if plan.observe {
        None
    } else {
        let (cols, rows) = tty::grid()?;
        Some(json!({"cols": cols, "rows": rows}))
    };
    let role = if plan.observe { "observe" } else { "interact" };
    let params = json!({
        "op_id": op_id(), "target": plan.target, "role": role, "client_id": plan.client,
        "geometry": geometry, "stream_mode": "plain", "takeover": null,
    });
    let opened: AttachOpenResult =
        serde_json::from_value(client.call("attach.open", params).await?)
            .map_err(unreadable("attach.open result"))?;
    let reason = || opened.reason.clone().unwrap_or_default();
    let refusal = match opened.outcome {
        OpenEnum::Known(AttachOutcome::Ready) => None,
        OpenEnum::Known(AttachOutcome::Ended) if plan.observe => None,
        OpenEnum::Known(AttachOutcome::Held) => Some(AttachError::Held {
            holder: serde_json::to_value(&opened.holder).unwrap_or(Value::Null),
        }),
        OpenEnum::Known(AttachOutcome::Ended) => Some(AttachError::Ended { reason: reason() }),
        OpenEnum::Known(AttachOutcome::Refused) => Some(AttachError::Refused { reason: reason() }),
        OpenEnum::Known(AttachOutcome::Recovering) => Some(AttachError::Recovering),
        _ => Some(AttachError::Protocol {
            detail: "unknown attach outcome",
        }),
    };
    let Some(refusal) = refusal else {
        return Ok(opened);
    };
    if let Some(stream) = &opened.stream_id {
        client
            .call("attach.close", json!({"stream_id": stream}))
            .await?;
    }
    Err(refusal)
}

pub(super) async fn geometry_lease(
    client: &Client,
    plan: &AttachPlan,
) -> Result<Option<String>, AttachError> {
    let leases = client
        .call("attach.leases", json!({"target": plan.target}))
        .await?;
    let owner = leases
        .get("geometry_owner")
        .filter(|owner| owner.get("holder").and_then(Value::as_str) == Some(plan.client.as_str()));
    Ok(owner
        .and_then(|owner| owner.get("lease_id"))
        .and_then(Value::as_str)
        .map(str::to_owned))
}
