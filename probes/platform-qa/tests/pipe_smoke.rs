//! Windows smoke over a real owner-only named pipe served by the production pipe
//! listener and connection core, guarded by the production boundary policy: two
//! clients at once, a foreign generation refused before any call, and an observer
//! refused every mutation while the owner is served.
#![cfg(windows)]

#[cfg(test)]
#[path = "pipe_smoke/served.rs"]
pub mod served;

use std::sync::atomic::Ordering;

use ovis_terminal_server::dispatch::lanes::boundary_policy::MUTATION_ROLES;
use serde_json::{Value, json};
use served::{Served, exchange, hello, line};
use tokio::net::windows::named_pipe::ClientOptions;

fn kind(reply: &Value) -> Option<&str> {
    reply.pointer("/error/data/kind").and_then(Value::as_str)
}

#[tokio::test]
async fn two_clients_are_served_concurrently_over_one_owner_only_pipe() {
    let served = Served::start();
    let first = ClientOptions::new().open(served.name.as_str()).unwrap();
    let second = ClientOptions::new().open(served.name.as_str()).unwrap();
    let get =
        |id: &str| line(&json!({"jsonrpc": "2.0", "id": id, "method": "doc.get", "params": {}}));
    let to_first = format!("{}{}", hello(&json!("1"), &served.owner), get("a"));
    let to_second = format!("{}{}", hello(&json!("1"), &served.owner), get("b"));
    let (a, b) = tokio::join!(
        exchange(first, &to_first, 2),
        exchange(second, &to_second, 2),
    );
    for (replies, id) in [(&a, "a"), (&b, "b")] {
        let (welcome, got) = (replies.first().unwrap(), replies.get(1).unwrap());
        assert!(
            welcome.pointer("/result/server_boot_id").is_some(),
            "{replies:?}"
        );
        assert_eq!(got.get("id"), Some(&json!(id)), "{replies:?}");
        assert_eq!(got.pointer("/result/served"), Some(&json!("doc.get")));
    }
    assert_eq!(served.reached.load(Ordering::SeqCst), 2);
    let report = served.server.shutdown().await.unwrap();
    assert_eq!((report.accepted(), report.failures().len()), (2, 0));
}

#[tokio::test]
async fn a_foreign_generation_is_refused_on_the_pipe_before_any_call() {
    let served = Served::start();
    let client = ClientOptions::new().open(served.name.as_str()).unwrap();
    let spawn = line(
        &json!({"jsonrpc": "2.0", "id": "spawn", "method": "terminal.run",
        "params": {"argv": ["cmd.exe", "/c", "echo SPAWNED"]}}),
    );
    let lines = format!(
        "{}{}{spawn}",
        hello(&json!(999_999), &served.owner),
        hello(&json!("999999"), &served.owner)
    );
    let replies = exchange(client, &lines, 3).await;
    let kinds: Vec<Option<&str>> = replies.iter().map(kind).collect();
    assert_eq!(
        kinds,
        [
            Some("invalid_params"),
            Some("unsupported_generation"),
            Some("unauthenticated")
        ],
        "{replies:?}"
    );
    assert!(replies.iter().all(|reply| reply.get("result").is_none()));
    assert_eq!(served.reached.load(Ordering::SeqCst), 0, "nothing ran");
    served.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_observer_is_refused_every_mutation_over_the_pipe_while_the_owner_is_served() {
    let served = Served::start();
    let observer = ClientOptions::new().open(served.name.as_str()).unwrap();
    let owner = ClientOptions::new().open(served.name.as_str()).unwrap();
    let mutations: Vec<&str> = MUTATION_ROLES
        .iter()
        .map(|(method, _)| *method)
        .filter(|method| *method != "attach.open")
        .collect();
    let mut lines = hello(&json!("1"), &served.observer);
    for method in &mutations {
        lines.push_str(&line(
            &json!({"jsonrpc": "2.0", "id": method, "method": method, "params": {}}),
        ));
    }
    let refused = exchange(observer, &lines, mutations.len().saturating_add(1)).await;
    let welcome = refused.first().unwrap();
    assert!(
        welcome.pointer("/result/server_boot_id").is_some(),
        "{refused:?}"
    );
    for reply in refused.iter().skip(1) {
        assert_eq!(kind(reply), Some("forbidden"), "{reply}");
        assert!(reply.get("result").is_none(), "{reply}");
    }
    assert_eq!(served.reached.load(Ordering::SeqCst), 0, "no mutation ran");
    let set = line(&json!({"jsonrpc": "2.0", "id": "set", "method": "doc.set", "params": {}}));
    let to_owner = format!("{}{set}", hello(&json!("1"), &served.owner));
    let owner_replies = exchange(owner, &to_owner, 2).await;
    let set_reply = owner_replies.get(1).unwrap();
    assert_eq!(
        set_reply.pointer("/result/served"),
        Some(&json!("doc.set")),
        "{owner_replies:?}"
    );
    assert_eq!(served.reached.load(Ordering::SeqCst), 1);
    served.server.shutdown().await.unwrap();
}
