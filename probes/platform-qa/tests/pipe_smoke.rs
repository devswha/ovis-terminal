//! Windows smoke over a real owner-only named pipe served by the production pipe
//! listener and connection core: a foreign generation is refused before any call. It
//! sends raw hello lines because the production CLI client always sends the canonical
//! generation; concurrent invocations and observer denial run through the production
//! client in `ovis-terminal-cli` `invocation::transport::pipe::tests`.
#![cfg(windows)]

#[cfg(test)]
#[path = "pipe_smoke/served.rs"]
pub mod served;

use std::sync::atomic::Ordering;

use serde_json::{Value, json};
use served::{Client, Served, TestResult, hello, line};

fn kind(reply: &Value) -> Option<&str> {
    reply.pointer("/error/data/kind").and_then(Value::as_str)
}

#[tokio::test]
async fn a_foreign_generation_is_refused_on_the_pipe_before_any_call() -> TestResult {
    let served = Served::start()?;
    let mut client = Client::open(&served.name)?;
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
