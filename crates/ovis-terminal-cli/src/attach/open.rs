//! `attach.open` with its typed outcomes, an explicit takeover bound to an epoch, and
//! the lease reads around them.

use ovis_terminal_protocol::schema::methods::attach::{AttachOpenResult, AttachOutcome};
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use serde_json::{Value, json};

use super::client::Client;
use super::{AttachError, AttachPlan, tty};

/// Reason recorded on both owners' lease events for a takeover this client asked for.
const TAKEOVER_REASON: &str =
    "explicit takeover requested with ovis-terminal terminal attach --takeover";

fn op_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

fn unreadable(detail: &'static str) -> impl FnOnce(serde_json::Error) -> AttachError {
    move |_| AttachError::Protocol { detail }
}

fn record<T: serde::Serialize>(value: Option<&T>) -> Value {
    value
        .and_then(|value| serde_json::to_value(value).ok())
        .unwrap_or(Value::Null)
}

fn params(plan: &AttachPlan) -> Result<Value, AttachError> {
    let geometry = if plan.observe {
        None
    } else {
        let (cols, rows) = tty::grid()?;
        Some(json!({"cols": cols, "rows": rows}))
    };
    let role = if plan.observe { "observe" } else { "interact" };
    let takeover = plan
        .takeover
        .map(|epoch| json!({"lease_epoch": epoch, "reason": TAKEOVER_REASON}));
    Ok(json!({
        "op_id": op_id(), "target": plan.target, "role": role, "client_id": plan.client,
        "geometry": geometry, "stream_mode": "plain", "takeover": takeover,
    }))
}

/// Opens the attach and turns a typed refusal into its error. A held lease is never
/// taken over: the observe stream it came with is closed again and the holder and the
/// lease change to wait for are reported. A takeover at a stale epoch reports the lease
/// that holds the slot now.
pub(super) async fn open(
    client: &Client,
    plan: &AttachPlan,
) -> Result<AttachOpenResult, AttachError> {
    let reply = client.call("attach.open", params(plan)?).await;
    let opened = match (reply, plan.takeover) {
        (Err(AttachError::Server(error)), Some(expected))
            if error.data.kind.token() == "lease_epoch_mismatch" =>
        {
            let leases = client
                .call("attach.leases", json!({"target": plan.target}))
                .await?;
            return Err(AttachError::EpochMismatch {
                expected: expected.get().to_string(),
                current: leases.get("interact").cloned().unwrap_or(Value::Null),
            });
        }
        (reply, _) => reply?,
    };
    let opened: AttachOpenResult =
        serde_json::from_value(opened).map_err(unreadable("attach.open result"))?;
    let reason = || opened.reason.clone().unwrap_or_default();
    let refusal = match opened.outcome {
        OpenEnum::Known(AttachOutcome::Ready) => None,
        OpenEnum::Known(AttachOutcome::Ended) if plan.observe => None,
        OpenEnum::Known(AttachOutcome::Held) => Some(AttachError::Conflict {
            holder: record(opened.holder.as_ref()),
            retry: record(opened.retry.as_ref()),
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

/// The geometry lease this client holds on the target, if any.
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

/// The leases among `granted` that the target still reports, in any slot.
pub(super) async fn still_held(
    client: &Client,
    plan: &AttachPlan,
    granted: &[String],
) -> Result<Vec<Value>, AttachError> {
    let leases = client
        .call("attach.leases", json!({"target": plan.target}))
        .await?;
    let slots = ["interact", "geometry_owner"]
        .iter()
        .filter_map(|slot| leases.get(*slot).filter(|lease| !lease.is_null()));
    let observers = leases
        .get("observers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    Ok(slots
        .chain(observers)
        .filter(|lease| {
            lease
                .get("lease_id")
                .and_then(Value::as_str)
                .is_some_and(|id| granted.iter().any(|mine| mine == id))
        })
        .cloned()
        .collect())
}
