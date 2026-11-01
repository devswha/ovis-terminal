use serde_json::{Value, json};

use super::{Client, Fixture, TestResult, op_id, publish};

const ZEROS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn at<'a>(value: &'a Value, pointer: &str) -> &'a Value {
    value.pointer(pointer).unwrap_or(&Value::Null)
}

fn stage(version: &str, sha256: &Value) -> Value {
    json!({"op_id": op_id(), "version": version,
        "target": ovis_terminal_protocol::domain::update::BuildTarget::host().as_str(), "sha256": sha256})
}

/// Stages, verifies and activates `version`; returns the activation reply and its op id.
async fn install(client: &mut Client, version: &str, digest: &str) -> TestResult<(Value, String)> {
    let staged = client
        .call("update.stage", &stage(version, &json!(digest)))
        .await?;
    let id = at(&staged, "/result/update_id").clone();
    assert_eq!(at(&staged, "/result/state"), "staged", "{staged}");
    let verified = client
        .call("update.verify", &json!({"op_id": op_id(), "update_id": id}))
        .await?;
    assert_eq!(at(&verified, "/result/state"), "verified", "{verified}");
    assert_eq!(
        at(&verified, "/result/build/manifest/protocol_generations"),
        &json!(["1"])
    );
    let op = op_id();
    let activated = client
        .call("update.activate", &json!({"op_id": op, "update_id": id}))
        .await?;
    Ok((activated, op))
}

async fn settled(client: &mut Client, op: &str) -> TestResult<Value> {
    let receipt = client.call("op.get", &json!({"op_id": op})).await?;
    Ok(at(&receipt, "/result").clone())
}

#[tokio::test(flavor = "multi_thread")]
async fn activation_restarts_control_and_the_held_program_keeps_running() -> TestResult {
    let fixture = Fixture::new().await?;
    let digest = publish(&fixture.root, "2.0.0", 0)?;
    let mut client = fixture.connect().await?;
    let (activated, _) = install(&mut client, "2.0.0", &digest).await?;
    assert_eq!(at(&activated, "/result/state"), "activated", "{activated}");
    let generation = json!([fixture.process.generation().get().to_string()]);
    assert_eq!(
        at(
            &activated,
            "/result/activation/restart/preserved_incarnations"
        ),
        &generation
    );
    assert_eq!(
        at(&activated, "/result/activation/restart/ended_incarnations"),
        &json!([])
    );
    assert_eq!(
        at(&activated, "/result/activation/readiness/outcome"),
        "ready"
    );
    let query = client.call("update.query", &json!({})).await?;
    assert_eq!(at(&query, "/result/active_version"), "2.0.0", "{query}");
    assert_eq!(at(&query, "/result/previous/version"), "1.0.0");
    assert_eq!(at(&query, "/result/live_incarnations"), &generation);
    assert!(fixture.child_running()?);
    drop(client);
    fixture.close().await
}

#[tokio::test(flavor = "multi_thread")]
async fn refusals_settle_failed_and_leave_the_active_build_and_programs() -> TestResult {
    let fixture = Fixture::new().await?;
    let mut client = fixture.connect().await?;
    let before = at(&client.call("update.query", &json!({})).await?, "/result").clone();
    publish(&fixture.root, "2.0.0", 0)?;
    let params = stage("2.0.0", &json!(ZEROS));
    let refused = client.call("update.stage", &params).await?;
    assert_eq!(
        at(&refused, "/error/data/kind"),
        "integrity_failed",
        "{refused}"
    );
    let op = at(&params, "/op_id").as_str().ok_or("op id")?.to_owned();
    let receipt = settled(&mut client, &op).await?;
    assert_eq!(at(&receipt, "/state"), "failed", "{receipt}");
    assert_eq!(at(&receipt, "/error/kind"), "integrity_failed");
    let numeric = client
        .call("update.stage", &stage("2.0.0", &json!(0)))
        .await?;
    assert_eq!(
        at(&numeric, "/error/data/kind"),
        "invalid_params",
        "{numeric}"
    );
    let digest = publish(&fixture.root, "3.0.0", 1)?;
    let (failed, op) = install(&mut client, "3.0.0", &digest).await?;
    assert_eq!(
        at(&failed, "/error/data/kind"),
        "activation_failed",
        "{failed}"
    );
    assert_eq!(at(&settled(&mut client, &op).await?, "/state"), "failed");
    let after = at(&client.call("update.query", &json!({})).await?, "/result").clone();
    assert_eq!(at(&after, "/active"), at(&before, "/active"), "{after}");
    assert_eq!(
        at(&after, "/live_incarnations"),
        at(&before, "/live_incarnations")
    );
    assert_eq!(at(&after, "/updates/0/state"), "rolled_back");
    assert_eq!(at(&after, "/updates/0/rollback/reason"), "readiness_failed");
    assert_eq!(at(&after, "/updates/0/rollback/readiness/exit_code"), 1);
    assert_eq!(at(&after, "/updates/0/rollback/restored/version"), "1.0.0");
    assert!(fixture.child_running()?);
    drop(client);
    fixture.close().await
}
