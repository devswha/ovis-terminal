//! `events.ack` on the real binary: an acknowledgement is recorded no further than the
//! sequence delivered on the subscription, never moves backwards, and only the
//! subscription's owner may make it.

use std::io::BufRead as _;

use serde_json::{Value, json};

use super::prompts::spawned_token;
use super::support::{Served, TestResult, op_id, rpc};

fn t1() -> Value {
    json!({"address": "t1"})
}

fn next_event(
    connection: &mut std::io::BufReader<std::os::unix::net::UnixStream>,
) -> TestResult<Value> {
    loop {
        let mut line = String::new();
        if connection.read_line(&mut line)? == 0 {
            return Err("the connection closed before the event".into());
        }
        let record: Value = serde_json::from_str(&line)?;
        if record.get("method") == Some(&json!("event")) {
            return Ok(record);
        }
    }
}

#[test]
fn events_ack_records_up_to_the_delivered_sequence_for_its_owner_only() -> TestResult {
    let served = Served::start()?;
    served.run_shell("shell")?;
    let token = spawned_token(&served)?;
    let mut subscriber = served.connected()?;
    let opened = rpc(
        &mut subscriber,
        "events.subscribe",
        &json!({"filter": {"terminals": null, "children_of": null, "labels": null,
            "types": ["status.changed"], "machines": null}, "cursor": null, "snapshot": false}),
    )?;
    let id = opened
        .pointer("/result/subscription_id")
        .cloned()
        .ok_or_else(|| opened.to_string())?;
    served.ok(&[
        "call",
        "status.report",
        "--params",
        &json!({"op_id": op_id()?,
        "target": t1(), "process_token": token, "seq": "1",
        "record": {"state": "working", "id": null, "kind": null, "progress": null,
            "app": null, "title": null, "msg": null}})
        .to_string(),
    ])?;
    let event = next_event(&mut subscriber)?;
    let seq: u64 = event
        .pointer("/params/seq")
        .and_then(Value::as_str)
        .ok_or_else(|| event.to_string())?
        .parse()?;

    let ahead = rpc(
        &mut subscriber,
        "events.ack",
        &json!({"subscription_id": id, "seq": (seq + 1).to_string()}),
    )?;
    assert_eq!(
        ahead.pointer("/error/data/kind"),
        Some(&json!("seq_ahead")),
        "{ahead}"
    );
    let acked = rpc(
        &mut subscriber,
        "events.ack",
        &json!({"subscription_id": id, "seq": seq.to_string()}),
    )?;
    assert_eq!(
        acked.pointer("/result/seq"),
        Some(&json!(seq.to_string())),
        "{acked}"
    );
    let scan: u64 = acked
        .pointer("/result/scan_seq")
        .and_then(Value::as_str)
        .ok_or_else(|| acked.to_string())?
        .parse()?;
    assert!(scan >= seq, "{acked}");
    let older = rpc(
        &mut subscriber,
        "events.ack",
        &json!({"subscription_id": id, "seq": "0"}),
    )?;
    assert_eq!(
        older.pointer("/result/seq"),
        Some(&json!(seq.to_string())),
        "{older}"
    );
    let numeric = rpc(
        &mut subscriber,
        "events.ack",
        &json!({"subscription_id": id, "seq": 0}),
    )?;
    assert_eq!(
        numeric.pointer("/error/data/kind"),
        Some(&json!("invalid_params")),
        "{numeric}"
    );
    let mut stranger = served.connected_as(served.observer())?;
    let refused = rpc(
        &mut stranger,
        "events.ack",
        &json!({"subscription_id": id, "seq": "0"}),
    )?;
    assert_eq!(
        refused.pointer("/error/data/kind"),
        Some(&json!("not_found")),
        "{refused}"
    );
    drop((subscriber, stranger));
    served.stop()?;
    Ok(())
}
