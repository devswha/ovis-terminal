//! Remote dispatch over a real socket to scripted machines: routing, no local fallback,
//! unreachable and lost-in-flight outcomes, pass-through refusals and local authorization.

mod harness;
mod peer;

use std::time::Duration;

use ovis_terminal_protocol::schema::methods::machine::MachineState;
use ovis_terminal_protocol::schema::scalar::{EventSeq, OpenEnum};
use serde_json::{Value, json};

use self::harness::{Harness, LOCAL, remote_id};
use self::peer::Mode;

pub(super) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Bound on waiting for any one event; never a pacing delay.
pub(super) const HANG: Duration = Duration::from_secs(20);

const DEADLINE: Duration = Duration::from_secs(20);

fn data<'a>(reply: &'a Value, field: &str) -> Option<&'a Value> {
    reply.pointer(&format!("/error/data/{field}"))
}

fn send(op: &str, address: &str) -> Value {
    json!({"op_id": op, "target": {"address": address}, "text": "ls", "submit": "none"})
}

#[tokio::test]
async fn a_qualified_target_reaches_only_its_machine_without_the_qualifier() -> TestResult {
    let harness = Harness::start(DEADLINE).await?;
    let mut peer = harness.machine(("box", 0xb1), Mode::Echo).await?;
    let mut client = harness.client(&harness.bearer).await?;

    let sent = client.call("input.send", &send("o1", "box/t1")).await?;
    let by_id = format!("{}/t1", remote_id(0xb1).as_str());
    let read = client
        .call("terminal.get", &json!({"target": {"address": by_id}}))
        .await?;
    let run = json!({"op_id": "o2", "program": {"argv": ["sh"]},
        "geometry": {"cols": "80", "rows": "24"}, "machine": {"name": "box"}});
    let launched = client.call("terminal.run", &run).await?;
    let unqualified = json!({"target": {"address": "t1"}});
    let own = json!({"target": {"address": format!("{LOCAL}/t1")}});
    let locally = client.call("terminal.get", &unqualified).await?;
    let own_machine = client.call("terminal.get", &own).await?;

    assert_eq!(sent.pointer("/result/method"), Some(&json!("input.send")));
    assert_eq!(sent.pointer("/result/params"), Some(&send("o1", "t1")));
    assert_eq!(
        read.pointer("/result/params/target/address"),
        Some(&json!("t1"))
    );
    assert_eq!(
        launched.pointer("/result/method"),
        Some(&json!("terminal.run"))
    );
    assert_eq!(launched.pointer("/result/params/machine"), None);
    assert_eq!(launched.pointer("/result/params/op_id"), Some(&json!("o2")));
    assert_eq!(
        locally.pointer("/result/local"),
        Some(&json!("terminal.get"))
    );
    assert_eq!(
        own_machine.pointer("/result/local"),
        Some(&json!("terminal.get"))
    );
    peer.read(3).await?;
    assert_eq!(peer.received().len(), 3);
    assert_eq!(harness.local.calls(), 2);
    harness.close().await
}

#[tokio::test]
async fn an_unknown_machine_is_refused_and_nothing_runs_locally() -> TestResult {
    let harness = Harness::start(DEADLINE).await?;
    let peer = harness.machine(("box", 0xb1), Mode::Echo).await?;
    let mut client = harness.client(&harness.bearer).await?;

    let addressed = client.call("input.send", &send("o1", "ghost/t1")).await?;
    let run = json!({"op_id": "o2", "program": {"argv": ["sh"]},
        "geometry": {"cols": "80", "rows": "24"}, "machine": {"name": "ghost"}});
    let launched = client.call("terminal.run", &run).await?;

    assert_eq!(data(&addressed, "kind"), Some(&json!("not_found")));
    assert_eq!(
        data(&addressed, "field_path"),
        Some(&json!("params.target.address"))
    );
    assert_eq!(data(&launched, "kind"), Some(&json!("not_found")));
    assert_eq!(
        data(&launched, "field_path"),
        Some(&json!("params.machine"))
    );
    assert_eq!(harness.local.calls(), 0);
    assert_eq!(peer.received(), Vec::<Value>::new());
    harness.close().await
}

#[tokio::test]
async fn an_unreachable_machine_refuses_with_its_last_seen_seq_and_sends_nothing() -> TestResult {
    let harness = Harness::start(DEADLINE).await?;
    let peer = harness.machine(("box", 0xb1), Mode::Echo).await?;
    let mut health = harness.actors.watch_health(&remote_id(0xb1))?;
    peer.event(&u64::MAX.to_string()).await?;
    let seen = health.wait_for(|health| health.last_seen_seq == Some(EventSeq::new(u64::MAX)));
    tokio::time::timeout(HANG, seen).await??;
    peer.cut().await;
    let unreachable = OpenEnum::Known(MachineState::Unreachable);
    tokio::time::timeout(HANG, health.wait_for(|health| health.state == unreachable)).await??;
    let mut client = harness.client(&harness.bearer).await?;

    let refused = client.call("input.send", &send("o1", "box/t1")).await?;

    assert_eq!(data(&refused, "kind"), Some(&json!("machine_unreachable")));
    assert_eq!(
        data(&refused, "last_seen_seq"),
        Some(&json!("18446744073709551615"))
    );
    assert_eq!(data(&refused, "target"), Some(&json!(remote_id(0xb1))));
    assert_eq!(data(&refused, "retryable"), Some(&json!(true)));
    assert_eq!(harness.local.calls(), 0);
    assert_eq!(peer.received(), Vec::<Value>::new());
    harness.close().await
}

#[tokio::test]
async fn a_mutation_lost_in_flight_is_outcome_unknown_and_a_lost_read_is_unreachable() -> TestResult
{
    let harness = Harness::start(DEADLINE).await?;
    let mut slow = harness.machine(("slow", 0xc1), Mode::Silent).await?;
    let mut quiet = harness.machine(("quiet", 0xc2), Mode::Silent).await?;
    let mut client = harness.client(&harness.bearer).await?;

    client
        .send("input.send", &send("o-lost", "slow/t1"))
        .await?;
    slow.read(1).await?;
    slow.cut().await;
    let lost = client.reply().await?;
    client
        .send("terminal.get", &json!({"target": {"address": "quiet/t1"}}))
        .await?;
    quiet.read(1).await?;
    quiet.cut().await;
    let read = client.reply().await?;

    assert_eq!(data(&lost, "kind"), Some(&json!("outcome_unknown")));
    assert_eq!(data(&lost, "op_id"), Some(&json!("o-lost")));
    assert_eq!(data(&lost, "target"), Some(&json!(remote_id(0xc1))));
    assert_eq!(data(&read, "kind"), Some(&json!("machine_unreachable")));
    assert_eq!(data(&read, "op_id"), None);
    assert_eq!(harness.local.calls(), 0);
    harness.close().await
}

#[tokio::test]
async fn the_machines_own_refusal_is_kept_and_a_deadline_names_the_operation() -> TestResult {
    let harness = Harness::start(Duration::from_millis(200)).await?;
    let _refusing = harness.machine(("box", 0xb1), Mode::Refuse).await?;
    let _stalled = harness.machine(("stall", 0xb2), Mode::Silent).await?;
    let mut client = harness.client(&harness.bearer).await?;

    let refused = client.call("input.send", &send("o1", "box/t1")).await?;
    let stalled = client.call("input.send", &send("o2", "stall/t1")).await?;

    assert_eq!(data(&refused, "kind"), Some(&json!("spawn_failed")));
    assert_eq!(data(&refused, "hint"), Some(&json!("fix the program")));
    assert_eq!(data(&stalled, "kind"), Some(&json!("deadline_exceeded")));
    assert_eq!(data(&stalled, "op_id"), Some(&json!("o2")));
    assert_eq!(harness.local.calls(), 0);
    harness.close().await
}

#[tokio::test]
async fn an_observer_may_read_a_machine_but_never_send_it_input() -> TestResult {
    let harness = Harness::start(DEADLINE).await?;
    let mut peer = harness.machine(("box", 0xb1), Mode::Echo).await?;
    let mut observer = harness.client(&harness.observer()?).await?;

    let denied = observer.call("input.send", &send("o1", "box/t1")).await?;
    let read = observer
        .call("terminal.get", &json!({"target": {"address": "box/t1"}}))
        .await?;

    assert_eq!(data(&denied, "kind"), Some(&json!("forbidden")));
    assert_eq!(read.pointer("/result/method"), Some(&json!("terminal.get")));
    peer.read(1).await?;
    let methods: Vec<Value> = peer
        .received()
        .iter()
        .filter_map(|request| request.get("method").cloned())
        .collect();
    assert_eq!(methods, vec![json!("terminal.get")]);
    harness.close().await
}

#[tokio::test]
async fn a_cancel_request_ends_only_the_named_call_and_the_connection_keeps_serving() -> TestResult
{
    let harness = Harness::start(DEADLINE).await?;
    let mut slow = harness.machine(("slow", 0xc1), Mode::Silent).await?;
    let mut client = harness.client(&harness.bearer).await?;

    client
        .send("input.send", &send("o-held", "slow/t1"))
        .await?;
    slow.read(1).await?;
    client
        .notify("$/cancelRequest", &json!({"id": "input.send"}))
        .await?;
    let cancelled = client.reply().await?;
    let after = client
        .call("terminal.get", &json!({"target": {"address": "t1"}}))
        .await?;

    assert_eq!(cancelled.get("id"), Some(&json!("input.send")));
    assert_eq!(data(&cancelled, "kind"), Some(&json!("cancelled")));
    assert_eq!(after.pointer("/result/local"), Some(&json!("terminal.get")));
    harness.close().await
}

#[test]
fn a_refusal_this_build_cannot_read_is_internal_never_a_success() {
    let numeric = json!({"code": -32000, "message": "x", "data": {
        "kind": "machine_unreachable", "hint": "h", "retryable": true, "last_seen_seq": 7,
    }});
    let typed = json!({"code": -32000, "message": "x", "data": {
        "kind": "machine_unreachable", "hint": "h", "retryable": true, "last_seen_seq": "7",
    }});

    assert_eq!(super::refusal::answered(&numeric).kind(), "internal");
    let kept = super::refusal::answered(&typed);
    assert_eq!(kept.kind(), "machine_unreachable");
    assert_eq!(
        serde_json::to_value(kept.data())
            .ok()
            .and_then(|data| data.get("last_seen_seq").cloned()),
        Some(json!("7"))
    );
}
