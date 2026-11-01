//! A machine's alias resolves to the same machine as its identity, and to no other.

use serde_json::{Value, json};

use super::harness::{Harness, remote_id};
use super::peer::Mode;
use super::{DEADLINE, TestResult};

#[tokio::test]
async fn an_alias_reaches_the_machine_it_names_and_only_that_machine() -> TestResult {
    let harness = Harness::start(DEADLINE).await?;
    let mut named = harness.machine(("box", 0xb1), Mode::Echo).await?;
    let other = harness.machine(("spare", 0xb2), Mode::Echo).await?;
    let mut client = harness.client(&harness.bearer).await?;

    let by_alias = client
        .call(
            "terminal.get",
            &json!({"target": {"address": "box-old/t1"}}),
        )
        .await?;
    let run = json!({"op_id": "o-alias", "program": {"argv": ["sh"]},
        "geometry": {"cols": "80", "rows": "24"}, "machine": {"name": "box-old"}});
    let launched = client.call("terminal.run", &run).await?;

    assert_eq!(
        by_alias.pointer("/result/params/target/address"),
        Some(&json!("t1"))
    );
    assert_eq!(
        launched.pointer("/result/method"),
        Some(&json!("terminal.run"))
    );
    named.read(2).await?;
    let methods: Vec<Value> = named
        .received()
        .iter()
        .filter_map(|request| request.get("method").cloned())
        .collect();
    assert_eq!(methods, vec![json!("terminal.get"), json!("terminal.run")]);
    assert_eq!(other.received(), Vec::<Value>::new());
    assert_eq!(harness.local.calls(), 0);
    assert_ne!(remote_id(0xb1), remote_id(0xb2));
    harness.close().await
}
