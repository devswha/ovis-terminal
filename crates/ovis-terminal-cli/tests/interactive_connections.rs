//! Plan todo 62: a lease belongs to the connection that acquired it. On a real `serve`, two
//! connections presenting one client id are two owners: the second is answered `held`
//! naming the holder and the retry boundary, and nothing it types, even under the holder's
//! lease id, reaches the program.
#![cfg(unix)]

#[path = "attach/support.rs"]
pub mod support;

#[path = "interactive_attach/helpers.rs"]
pub mod helpers;

#[path = "interactive_connections/socket.rs"]
pub mod socket;

use base64::Engine as _;
use helpers::incarnation;
use serde_json::{Value, json};
use socket::Socket;
use support::{Served, TestResult, WAIT};

fn interact_as_sdk() -> Value {
    json!({
        "op_id": uuid::Uuid::now_v7().to_string(), "target": {"address": "t1"},
        "role": "interact", "client_id": "sdk", "geometry": null,
        "stream_mode": "plain", "takeover": null,
    })
}

fn typed(text: &str, lease: &Value) -> Value {
    json!({
        "op_id": uuid::Uuid::now_v7().to_string(), "target": {"address": "t1"}, "expect": null,
        "data_b64": base64::engine::general_purpose::STANDARD.encode(text), "lease_id": lease,
    })
}

/// The program's raw output from its first byte, as text.
fn program_output(served: &Served) -> TestResult<String> {
    let read = served.call(
        "terminal.output",
        &json!({"target": {"address": "t1"}, "from_offset": "0", "limit_bytes": "65536"}),
    )?;
    let encoded = read.get("data_b64").and_then(Value::as_str);
    let encoded = encoded.ok_or_else(|| format!("no data in {read}"))?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[test]
fn two_connections_under_one_client_id_compete_and_the_second_never_types_into_the_program()
-> TestResult {
    let served = Served::start()?;
    let before = incarnation(&served)?;
    let mut first = Socket::open(&served)?;
    let mut second = Socket::open(&served)?;
    let held = first.result("attach.open", &interact_as_sdk())?;
    assert_eq!(held.get("outcome"), Some(&json!("ready")), "{held}");
    let lease = held.pointer("/lease/lease_id").cloned();
    let lease = lease.ok_or_else(|| format!("no interact lease in {held}"))?;

    let refused = second.result("attach.open", &interact_as_sdk())?;
    assert_eq!(
        refused.get("outcome"),
        Some(&json!("held")),
        "the second connection shared the holder's lease: {refused}"
    );
    assert_eq!(refused.pointer("/holder/holder"), Some(&json!("sdk")));
    assert_eq!(refused.pointer("/holder/lease_id"), Some(&lease));
    assert_eq!(refused.pointer("/retry/on"), Some(&json!("lease_released")));
    assert_eq!(
        refused.pointer("/retry/event"),
        Some(&json!("lease.revoked"))
    );
    assert_eq!(refused.pointer("/lease/role"), Some(&json!("observe")));

    let borrowed = second.reply("input.raw", &typed("echo intruder-$((3+4))\r", &lease))?;
    assert_eq!(
        borrowed.pointer("/error/data/kind"),
        Some(&json!("no_lease")),
        "{borrowed}"
    );
    // The refusal was answered before the holder's fence is sent, and the program reads its
    // input in order, so typing that reached it would be printed before the fence's answer.
    first.result("input.raw", &typed("echo fence-$((5+5))\r", &lease))?;
    let waited = served.call(
        "terminal.wait_output",
        &json!({
            "target": {"address": "t1"}, "cursor": {"offset": "0"},
            "pattern": {"text": "fence-10"}, "timeout_ms": WAIT.as_millis().to_string(),
        }),
    )?;
    assert_eq!(waited.get("outcome"), Some(&json!("matched")), "{waited}");
    assert!(
        !program_output(&served)?.contains("intruder-7"),
        "the refused connection typed into the program"
    );
    let leases = served.leases()?;
    assert_eq!(
        leases.pointer("/interact/lease_id"),
        Some(&lease),
        "{leases}"
    );
    assert_eq!(incarnation(&served)?, before);
    drop((first, second));
    served.stop()
}
