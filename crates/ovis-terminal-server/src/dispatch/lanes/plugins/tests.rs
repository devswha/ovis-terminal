use ovis_terminal_authz::Permission;
use serde_json::{Value, json};
use support::{Harness, TestResult, at, op_id};

mod support;

fn sh(id: &str, script: &str) -> Value {
    json!({"action_id": id, "argv": ["/bin/sh", format!("$ROOT/plugins/fixture/{script}")],
           "contexts": ["machine"]})
}

fn fixture() -> [(&'static str, Value); 2] {
    [
        (
            "fixture",
            json!({"plugin_id": "fixture", "version": "1.0.0", "enabled": true,
            "actions": [sh("report", "report.sh"), sh("admin-action", "admin.sh"), sh("hang", "hang.sh")]}),
        ),
        (
            "off",
            json!({"plugin_id": "off", "version": "0.1.0", "enabled": false,
            "actions": [{"action_id": "noop", "argv": ["/bin/sh"], "contexts": ["machine"]}]}),
        ),
    ]
}

async fn start(name: &str) -> TestResult<Harness> {
    let harness = Harness::start(name, &fixture()).await?;
    let scripts = harness.root.join("plugins").join("fixture");
    std::fs::write(scripts.join("report.sh"), "printf 'OK\\n'\nexit 7\n")?;
    std::fs::write(
        scripts.join("admin.sh"),
        "printf spawned > \"$0.spawned\"\n",
    )?;
    std::fs::write(
        scripts.join("hang.sh"),
        "printf started > \"$0.fifo\"\nexec sleep 600\n",
    )?;
    Ok(harness)
}

fn invoke(op: &str, plugin: &str, action: &str) -> Value {
    json!({"op_id": op, "plugin_id": plugin, "action_id": action, "input": {}, "target": null})
}

#[tokio::test(flavor = "multi_thread")]
async fn printed_ok_then_exit_7_is_retained_as_a_failed_run() -> TestResult {
    let harness = start("happy").await?;
    let mut client = harness.connect(&harness.owner.clone()).await?;
    let op = op_id();

    let listed = client.call("plugin.list", json!({})).await?;
    let invoked = client
        .call("plugin.invoke", invoke(&op, "fixture", "report"))
        .await?;
    let run_id = at(&invoked, "/result/run_id").clone();
    let fetched = client
        .call("plugin.run_get", json!({"run_id": run_id}))
        .await?;
    let receipt = client.call("op.get", json!({"op_id": op})).await?;

    assert_eq!(
        at(&listed, "/result/plugins/1/actions/0/unavailable"),
        json!("plugin_disabled")
    );
    assert_eq!(
        at(&listed, "/result/plugins/0/actions/0/unavailable"),
        Value::Null
    );
    let run = at(&invoked, "/result");
    assert_eq!(at(&run, "/state"), json!("failed"));
    assert_eq!(at(&run, "/exit_code"), json!(7));
    assert_eq!(at(&run, "/output_b64"), json!("T0sK"));
    assert_eq!(at(&run, "/output_floor"), json!("0"));
    assert_eq!(at(&run, "/truncated"), json!(false));
    assert_eq!(at(&run, "/created_terminals"), json!([]));
    assert_eq!(run_id, json!(format!("plugin-run:{op}")));
    assert_eq!(at(&fetched, "/result"), run);
    assert_eq!(at(&receipt, "/result/state"), json!("committed"));
    assert_eq!(at(&receipt, "/result/result"), run);
    drop(client);
    harness.close().await
}

#[tokio::test(flavor = "multi_thread")]
async fn an_observer_is_forbidden_before_any_program_starts() -> TestResult {
    let harness = start("forbidden").await?;
    let observer = harness.issue("observer", Permission::Observe)?;
    let mut watcher = harness.connect(&observer).await?;
    let mut owner = harness.connect(&harness.owner.clone()).await?;
    let (denied_op, disabled_op) = (op_id(), op_id());

    let denied = watcher
        .call(
            "plugin.invoke",
            invoke(&denied_op, "fixture", "admin-action"),
        )
        .await?;
    let disabled = owner
        .call("plugin.invoke", invoke(&disabled_op, "off", "noop"))
        .await?;
    let disabled_receipt = owner.call("op.get", json!({"op_id": disabled_op})).await?;
    let foreign = watcher
        .call(
            "plugin.run_get",
            json!({"run_id": format!("plugin-run:{disabled_op}")}),
        )
        .await?;
    let listed = watcher.call("plugin.list", json!({})).await?;

    assert_eq!(at(&denied, "/error/data/kind"), json!("forbidden"));
    let marker = harness.root.join("plugins/fixture/admin.sh.spawned");
    assert!(!marker.exists(), "the refused action ran");
    assert_eq!(at(&disabled, "/error/data/kind"), json!("plugin_disabled"));
    assert_eq!(at(&disabled_receipt, "/result/state"), json!("failed"));
    assert_eq!(
        at(&disabled_receipt, "/result/error/kind"),
        json!("plugin_disabled")
    );
    assert_eq!(at(&foreign, "/error/data/kind"), json!("not_found"));
    assert_eq!(
        at(&listed, "/result/plugins/1/actions/0/unavailable"),
        json!("plugin_disabled")
    );
    drop((watcher, owner));
    harness.close().await
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_hung_run_kills_it_and_commits_cancelled() -> TestResult {
    let harness = start("hung").await?;
    let fifo = harness.root.join("plugins/fixture/hang.sh.fifo");
    let made = std::process::Command::new("mkfifo").arg(&fifo).status()?;
    assert!(made.success(), "mkfifo: {made}");
    let mut client = harness.connect(&harness.owner.clone()).await?;
    let op = op_id();

    client
        .send(
            &json!({"jsonrpc": "2.0", "id": "invoke", "method": "plugin.invoke",
                      "params": invoke(&op, "fixture", "hang")}),
        )
        .await?;
    // The program writes the FIFO only once it runs, i.e. after the ledger marked it running.
    let started = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        tokio::task::spawn_blocking(move || std::fs::read(fifo)),
    )
    .await???;
    client
        .send(&json!({"jsonrpc": "2.0", "id": "cancel", "method": "op.cancel", "params": {"op_id": op}}))
        .await?;
    let mut replies = [client.next().await?, client.next().await?];
    replies.sort_by_key(|reply| at(reply, "/id").as_str().map(str::to_owned));
    let [cancelled, invoked] = replies;
    let receipt = client.call("op.get", json!({"op_id": op})).await?;

    assert_eq!(started, b"started");
    assert_eq!(at(&cancelled, "/result/outcome"), json!("pending"));
    assert_eq!(at(&invoked, "/result/state"), json!("cancelled"));
    assert_eq!(at(&invoked, "/result/exit_code"), Value::Null);
    assert_eq!(
        at(&receipt, "/result/evidence/cancellation_requested"),
        json!(true)
    );
    assert_eq!(at(&receipt, "/result/result"), at(&invoked, "/result"));
    drop(client);
    harness.close().await
}
