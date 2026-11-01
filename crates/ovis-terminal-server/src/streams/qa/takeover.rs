//! `attach.takeover` moves the interact lease onto the caller's held stream.
//!
//! A caller without the takeover grant and a stale epoch change nothing; the takeover
//! closes the holder's stream `taken_over` naming the new lease and tells the caller
//! input is ready.

use ovis_terminal_authz::Permission;
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_server::streams::StreamLimits;
use serde_json::{Value, json};

use super::TestResult;
use super::fixture::{Fixture, TERMINAL};
use super::socket::{at, error_kind};
use super::viewer::{Viewer, open_params};

const TAKER: &[Permission] = &[
    Permission::Observe,
    Permission::Interact,
    Permission::Takeover,
];
const INTERACTOR: &[Permission] = &[Permission::Observe, Permission::Interact];

fn takeover(op: &str, stream: &str, epoch: &Value) -> Value {
    json!({"op_id": op, "target": {"terminal_id": TERMINAL}, "stream_id": stream,
        "takeover": {"lease_epoch": epoch, "reason": "operator handoff"}})
}

/// The whole scenario on one fixture.
pub(crate) async fn takeover_moves_the_interact_lease_onto_the_held_stream() -> TestResult {
    let mut fx = Fixture::new(StreamLimits::advertised(&Limits::v1()), 64 * 1024 * 1024).await?;
    fx.issue("bridge", TAKER)?;
    fx.issue("phone", INTERACTOR)?;
    fx.publish(b"ready\r\n")?;
    let owner = fx.connect("owner").await?;
    let mut holder = Viewer::open(owner, &open_params("op-1", "tui", "interact"), true).await?;
    holder
        .until(|viewer| !viewer.kinds("input_ready").is_empty())
        .await?;
    let bridge = fx.connect("bridge").await?;
    let mut held = Viewer::open(bridge, &open_params("op-2", "bridge", "interact"), true).await?;
    assert_eq!(
        at(&held.result, "/result/outcome"),
        "held",
        "{}",
        held.result
    );
    let epoch = at(&held.result, "/result/holder/epoch").clone();
    let phone = fx.connect("phone").await?;
    let mut phone = Viewer::open(phone, &open_params("op-3", "phone", "interact"), true).await?;
    let stream = phone.stream.clone();
    let refused = phone
        .call("attach.takeover", &takeover("op-4", &stream, &epoch))
        .await?;
    assert_eq!(error_kind(&refused), "forbidden", "{refused}");
    let stale = held.stream.clone();
    let stale = held
        .call("attach.takeover", &takeover("op-5", &stale, &json!("99")))
        .await?;
    assert_eq!(error_kind(&stale), "lease_epoch_mismatch", "{stale}");
    let stream = held.stream.clone();
    let taken = held
        .call("attach.takeover", &takeover("op-6", &stream, &epoch))
        .await?;
    assert_eq!(at(&taken, "/result/acquired/holder"), "bridge", "{taken}");
    assert_eq!(at(&taken, "/result/acquired/role"), "interact");
    assert_eq!(at(&taken, "/result/displaced/holder"), "tui");
    assert!(at(&taken, "/result/seq").is_string(), "{taken}");
    held.until(|viewer| !viewer.kinds("input_ready").is_empty())
        .await?;
    holder
        .until(|viewer| !viewer.kinds("close").is_empty())
        .await?;
    let close = holder.first("close")?;
    assert_eq!(at(&close, "/params/close/reason"), "taken_over", "{close}");
    assert_eq!(
        at(&close, "/params/close/by"),
        at(&taken, "/result/acquired")
    );
    let replay = held
        .call("attach.takeover", &takeover("op-6", &stream, &epoch))
        .await?;
    assert_eq!(
        at(&replay, "/result"),
        at(&taken, "/result"),
        "a replay answers once"
    );
    fx.close().await
}
