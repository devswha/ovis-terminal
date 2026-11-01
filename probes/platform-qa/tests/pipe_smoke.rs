//! Windows smoke over a real owner-only named pipe served by the production pipe
//! listener and connection core, guarded by the production boundary policy: a foreign
//! generation refused before any call, and an observer refused every mutation while
//! the owner is served. Two concurrent production CLI invocations are proven by
//! `ovis-terminal-cli` `invocation::transport::pipe::tests`; this helper client
//! waits out a busy instance the same way the CLI client does.
#![cfg(windows)]

#[cfg(test)]
#[path = "pipe_smoke/served.rs"]
pub mod served;

use std::sync::atomic::Ordering;

use ovis_terminal_server::dispatch::lanes::boundary_policy::MUTATION_ROLES;
use serde_json::{Value, json};
use served::{Client, Served, TestResult, hello, line};

fn kind(reply: &Value) -> Option<&str> {
    reply.pointer("/error/data/kind").and_then(Value::as_str)
}

fn nth(replies: &[Value], index: usize) -> TestResult<&Value> {
    Ok(replies
        .get(index)
        .ok_or_else(|| format!("no reply {index} in {replies:?}"))?)
}

#[tokio::test]
async fn a_foreign_generation_is_refused_on_the_pipe_before_any_call() -> TestResult {
    let served = Served::start();
    let mut client = Client::open(&served.name).await?;
    let spawn = line(
        &json!({"jsonrpc": "2.0", "id": "spawn", "method": "terminal.run",
        "params": {"argv": ["cmd.exe", "/c", "echo SPAWNED"]}}),
    );
    let lines = format!(
        "{}{}{spawn}",
        hello(&json!(999_999), &served.owner),
        hello(&json!("999999"), &served.owner)
    );
    let replies = client.exchange(&lines, 3).await?;
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
    drop(client);
    served.server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn an_observer_is_refused_every_mutation_over_the_pipe_while_the_owner_is_served()
-> TestResult {
    let served = Served::start();
    let (observer, owner) = tokio::join!(Client::open(&served.name), Client::open(&served.name));
    let (mut observer, mut owner) = (observer?, owner?);
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
    let refused = observer
        .exchange(&lines, mutations.len().saturating_add(1))
        .await?;
    assert!(
        nth(&refused, 0)?
            .pointer("/result/server_boot_id")
            .is_some(),
        "{refused:?}"
    );
    for reply in refused.iter().skip(1) {
        assert_eq!(kind(reply), Some("forbidden"), "{reply}");
        assert!(reply.get("result").is_none(), "{reply}");
    }
    assert_eq!(served.reached.load(Ordering::SeqCst), 0, "no mutation ran");
    let set = line(&json!({"jsonrpc": "2.0", "id": "set", "method": "doc.set", "params": {}}));
    let to_owner = format!("{}{set}", hello(&json!("1"), &served.owner));
    let owner_replies = owner.exchange(&to_owner, 2).await?;
    assert_eq!(
        nth(&owner_replies, 1)?.pointer("/result/served"),
        Some(&json!("doc.set")),
        "{owner_replies:?}"
    );
    assert_eq!(served.reached.load(Ordering::SeqCst), 1);
    drop((observer, owner));
    served.server.shutdown().await?;
    Ok(())
}
