//! Typed rejections: credit, stale streams, upstream DATA, refusal, recovery, disconnect.

use ovis_terminal_authz::{Permission, PrincipalId};
use ovis_terminal_protocol::schema::ids::StreamId;
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_protocol::wire::{Frame, FrameKind};
use ovis_terminal_server::streams::{StreamChannels, StreamLimits, StreamOffset};
use ovis_terminal_store::OpId;
use serde_json::{Value, json};

use super::TestResult;
use super::fixture::Fixture;
use super::socket::{at, error_kind};
use super::viewer::{Viewer, open_params};

const OBSERVER: &[Permission] = &[Permission::Observe];
const INTERACTOR: &[Permission] = &[Permission::Observe, Permission::Interact];

fn outcome(viewer: &Viewer) -> &Value {
    at(&viewer.result, "/result/outcome")
}

fn credit(stream: &str, acknowledged: &Value) -> Value {
    json!({"stream_id": stream, "acknowledged": acknowledged})
}

/// Credit beyond sent bytes, regressing, foreign, numeric and for a closed stream.
async fn refused_credit(fx: &Fixture) -> TestResult {
    let mut viewer = Viewer::open(
        fx.connect("owner").await?,
        &open_params("c-1", "tui", "observe"),
        false,
    )
    .await?;
    viewer.until(|viewer| !viewer.received.is_empty()).await?;
    let sent = viewer.received.len();
    let beyond = viewer
        .call(
            "attach.credit",
            &credit(&viewer.stream, &json!(sent.saturating_add(1).to_string())),
        )
        .await?;
    assert_eq!(error_kind(&beyond), "invalid_credit");
    let maximum = viewer
        .call(
            "attach.credit",
            &credit(&viewer.stream, &json!(u64::MAX.to_string())),
        )
        .await?;
    assert_eq!(error_kind(&maximum), "invalid_credit");
    let numeric = viewer
        .call("attach.credit", &credit(&viewer.stream, &json!(sent)))
        .await?;
    assert_eq!(
        error_kind(&numeric),
        "invalid_params",
        "offsets are decimal strings"
    );
    // Then nothing was granted: acknowledging exactly what was sent opens the full window.
    let owner = PrincipalId::parse("owner")?;
    let stream = StreamId::new(viewer.stream.clone());
    let at_sent = StreamOffset::new(u64::try_from(sent)?);
    assert_eq!(fx.lane.acknowledge(&owner, (&stream, at_sent))?, 256 * 1024);
    let regressed = viewer
        .call("attach.credit", &credit(&viewer.stream, &json!("0")))
        .await?;
    assert_eq!(error_kind(&regressed), "invalid_credit");
    let mut stranger = Viewer::open(
        fx.connect("other").await?,
        &open_params("c-2", "stranger", "observe"),
        true,
    )
    .await?;
    let foreign = stranger
        .call("attach.credit", &credit(&viewer.stream, &json!("0")))
        .await?;
    assert_eq!(
        error_kind(&foreign),
        "stream_not_found",
        "another principal's stream is invisible"
    );
    // When the stream closes, its identity never grants credit again.
    viewer
        .call("attach.close", &json!({"stream_id": viewer.stream}))
        .await?;
    viewer
        .until(|viewer| !viewer.kinds("close").is_empty())
        .await?;
    let old = viewer
        .call(
            "attach.credit",
            &credit(&viewer.stream, &json!(sent.to_string())),
        )
        .await?;
    assert_eq!(error_kind(&old), "invalid_credit");
    Ok(())
}

/// Upstream frames on a stream channel: DATA is refused before anything is written.
fn upstream_frames() -> TestResult {
    let mut channels = StreamChannels::default();
    let old = channels.bind(StreamId::new("stream:old".to_owned()))?;
    channels.retire(old);
    let live = channels.bind(StreamId::new("stream:new".to_owned()))?;
    let data = Frame::new(live, FrameKind::Data, b"\x03rm -rf ~\r".to_vec())?;
    assert_eq!(
        channels.upstream(&data).map_err(|error| error.kind()),
        Err("direction_violation")
    );
    let stale = Frame::new(old, FrameKind::Credit, 1_u64.to_le_bytes().to_vec())?;
    assert_eq!(
        channels.upstream(&stale).map_err(|error| error.kind()),
        Err("invalid_credit")
    );
    Ok(())
}

pub(crate) async fn failure() -> TestResult {
    let mut fx = Fixture::new(StreamLimits::advertised(&Limits::v1()), 64 * 1024 * 1024).await?;
    fx.issue("watcher", OBSERVER)?;
    fx.issue("other", INTERACTOR)?;
    fx.publish(b"$ ")?;
    refused_credit(&fx).await?;
    upstream_frames()?;
    // An observe-only credential asking for input is refused as a record, with no stream.
    let refused = Viewer::open(
        fx.connect("watcher").await?,
        &open_params("f-1", "watcher", "interact"),
        true,
    )
    .await?;
    assert_eq!(outcome(&refused), "refused");
    assert_eq!(at(&refused.result, "/result/reason"), "forbidden");
    assert!(at(&refused.result, "/result/stream_id").is_null());
    // The same op_id with other params is op_reused; with the same params, the same result.
    let mut again = Viewer::open(
        fx.connect("watcher").await?,
        &open_params("f-1", "watcher", "observe"),
        true,
    )
    .await?;
    assert_eq!(error_kind(&again.result), "op_reused");
    let replay = again
        .call("attach.open", &open_params("f-1", "watcher", "interact"))
        .await?;
    assert_eq!(replay.get("result"), refused.result.get("result"));
    // Disconnect revokes the interact lease, so the next interact attach is ready.
    let gone = Viewer::open(
        fx.connect("other").await?,
        &open_params("f-2", "other", "interact"),
        true,
    )
    .await?;
    assert_eq!(outcome(&gone), "ready");
    drop(gone);
    fx.journal.revoked("other").await?;
    let mut ready = Viewer::open(
        fx.connect("owner").await?,
        &open_params("f-3", "tui", "interact"),
        true,
    )
    .await?;
    assert_eq!(outcome(&ready), "ready");
    ready
        .until(|viewer| !viewer.kinds("input_ready").is_empty())
        .await?;
    // Recovery revokes readiness before CLOSE `lost`, and attaches wait for recovery.
    let hub = std::sync::Arc::clone(&fx.hub);
    tokio::task::spawn_blocking(move || hub.recovering(&OpId::new("recovery-1"))).await??;
    ready
        .until(|viewer| !viewer.kinds("close").is_empty())
        .await?;
    let kinds: Vec<&Value> = ready
        .controls
        .iter()
        .map(|record| at(record, "/params/kind"))
        .collect();
    let tail: Vec<&Value> = kinds.iter().rev().take(2).rev().copied().collect();
    assert_eq!(
        tail,
        [&json!("input_revoked"), &json!("close")],
        "{kinds:?}"
    );
    assert_eq!(at(&ready.first("close")?, "/params/close/reason"), "lost");
    let waiting = Viewer::open(
        fx.connect("owner").await?,
        &open_params("f-4", "tui", "interact"),
        true,
    )
    .await?;
    assert_eq!(outcome(&waiting), "recovering");
    assert_eq!(at(&waiting.result, "/result/retry/on"), "recovered");
    assert_eq!(
        at(&waiting.result, "/result/retry/event"),
        "server.recovery"
    );
    assert!(at(&waiting.result, "/result/stream_id").is_null());
    fx.hub.recovered()?;
    let mut fresh = Viewer::open(
        fx.connect("owner").await?,
        &open_params("f-5", "tui", "interact"),
        true,
    )
    .await?;
    assert_eq!(outcome(&fresh), "ready");
    assert_ne!(
        fresh.stream, ready.stream,
        "a recovered terminal never reuses a stream identity"
    );
    planned_restart(&fx, &mut fresh).await?;
    fx.close().await
}

/// A planned restart closes the stream `server_restart`, never `lost`; the successor
/// serves the same terminal on a new stream identity.
async fn planned_restart(fx: &Fixture, live: &mut Viewer) -> TestResult {
    let hub = std::sync::Arc::clone(&fx.hub);
    tokio::task::spawn_blocking(move || hub.restarting(&OpId::new("restart-1"))).await??;
    live.until(|viewer| !viewer.kinds("close").is_empty())
        .await?;
    assert_eq!(
        at(&live.first("close")?, "/params/close/reason"),
        "server_restart"
    );
    let waiting = Viewer::open(
        fx.connect("owner").await?,
        &open_params("f-6", "tui", "interact"),
        true,
    )
    .await?;
    assert_eq!(outcome(&waiting), "recovering");
    fx.hub.recovered()?;
    let successor = Viewer::open(
        fx.connect("owner").await?,
        &open_params("f-7", "tui", "interact"),
        true,
    )
    .await?;
    assert_eq!(outcome(&successor), "ready");
    assert_ne!(
        successor.stream, live.stream,
        "a restart never reuses a stream identity"
    );
    Ok(())
}
