//! `events.subscribe`, `events.update` and `events.unsubscribe` on the real binary: a
//! subscription filtered to `status.changed` receives the program's declared status as
//! an `event` notification on its own connection, another principal cannot close it, and
//! its owner's unsubscribe reports where delivery stopped.

use std::io::BufRead as _;

use serde_json::{Value, json};

use super::prompts::spawned_token;
use super::support::{Served, TestResult, op_id, rpc};

fn filter(types: &[&str]) -> Value {
    json!({"terminals": null, "children_of": null, "labels": null, "types": types,
        "machines": null})
}

#[test]
fn a_subscription_receives_declared_status_and_only_its_owner_closes_it() -> TestResult {
    let served = Served::start()?;
    served.run_shell("shell")?;
    let token = spawned_token(&served)?;
    let mut subscriber = served.connected()?;
    let opened = rpc(
        &mut subscriber,
        "events.subscribe",
        &json!({"filter": filter(&["status.changed"]), "cursor": null, "snapshot": true}),
    )?;
    let id = opened
        .pointer("/result/subscription_id")
        .cloned()
        .ok_or_else(|| opened.to_string())?;
    let snapshot = opened.pointer("/result/snapshot").and_then(Value::as_array);
    assert_eq!(
        snapshot.map(Vec::len),
        Some(1),
        "the spawned terminal: {opened}"
    );

    served.ok(&[
        "call",
        "status.report",
        "--params",
        &json!({"op_id": op_id()?,
        "target": {"address": "t1"}, "process_token": token, "seq": "1",
        "record": {"state": "working", "id": null, "kind": null, "progress": null,
            "app": null, "title": null, "msg": null}})
        .to_string(),
    ])?;
    let event = loop {
        let mut line = String::new();
        if subscriber.read_line(&mut line)? == 0 {
            return Err("the connection closed before the event".into());
        }
        let record: Value = serde_json::from_str(&line)?;
        if record.get("method") == Some(&json!("event")) {
            break record;
        }
    };
    assert_eq!(
        event.pointer("/params/type"),
        Some(&json!("status.changed")),
        "{event}"
    );
    assert_eq!(
        event.pointer("/params/subscription_id"),
        Some(&id),
        "{event}"
    );

    let updated = rpc(
        &mut subscriber,
        "events.update",
        &json!({"subscription_id": id, "filter": filter(&["doc.changed"])}),
    )?;
    assert!(
        updated.pointer("/result/effective_seq").is_some(),
        "{updated}"
    );
    let mut stranger = served.connected_as(served.observer())?;
    let refused = rpc(
        &mut stranger,
        "events.unsubscribe",
        &json!({"subscription_id": id}),
    )?;
    assert_eq!(
        refused.pointer("/error/data/kind"),
        Some(&json!("not_found")),
        "{refused}"
    );
    let closed = rpc(
        &mut subscriber,
        "events.unsubscribe",
        &json!({"subscription_id": id}),
    )?;
    assert!(closed.pointer("/result/scan_seq").is_some(), "{closed}");
    let again = rpc(
        &mut subscriber,
        "events.unsubscribe",
        &json!({"subscription_id": id}),
    )?;
    assert_eq!(
        again.pointer("/error/data/kind"),
        Some(&json!("not_found")),
        "{again}"
    );
    drop((subscriber, stranger));
    served.stop()?;
    Ok(())
}
