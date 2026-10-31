//! Two isolated runtimes: the local one adds the remote one over a real argv transport.

mod local;
mod remote;

use std::time::Duration;

use local::{Client, Local};
use remote::{BOOT, MACHINE, Remote};
use serde_json::{Value, json};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const BOUND: Duration = Duration::from_secs(20);

fn op() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// `machine.add` params trusting the remote machine.
fn add(name: &str, transport: &[String], credential: &str) -> Value {
    json!({
        "op_id": op(), "name": name, "transport": {"argv": transport},
        "credential": credential, "expected_machine_id": MACHINE,
    })
}

fn expecting(mut params: Value, expected: Value) -> Value {
    if let Some(object) = params.as_object_mut() {
        object.insert("expected_machine_id".to_owned(), expected);
    }
    params
}

fn kind(reply: &Value) -> &str {
    reply
        .pointer("/error/data/kind")
        .and_then(Value::as_str)
        .unwrap_or("<no error>")
}

async fn get(client: &mut Client, reference: Value) -> TestResult<Value> {
    client
        .call("machine.get", &json!({"machine": reference}))
        .await
}

#[tokio::test]
async fn a_machine_keeps_its_destination_and_proven_boot_across_a_rename() -> TestResult {
    let mut remote = Remote::start()?;
    let local = Local::start(BOUND).await?;
    let mut client = local.client(&local.bearer).await?;

    let added = client
        .call("machine.add", &add("box", &remote.relay()?, &remote.bearer))
        .await?;
    let renamed = client
        .call(
            "machine.rename",
            &json!({"op_id": op(), "machine": {"name": "box"}, "name": "box2"}),
        )
        .await?;

    assert_eq!(
        added.pointer("/result/machine_id"),
        Some(&json!(MACHINE)),
        "{added}"
    );
    assert_eq!(added.pointer("/result/remote_boot_id"), Some(&json!(BOOT)));
    assert_eq!(added.pointer("/result/state"), Some(&json!("connected")));
    assert_eq!(added.pointer("/result/revision"), Some(&json!("1")));
    assert_eq!(
        renamed.pointer("/result/name"),
        Some(&json!("box2")),
        "{renamed}"
    );
    assert_eq!(renamed.pointer("/result/aliases"), Some(&json!(["box"])));
    assert_eq!(renamed.pointer("/result/revision"), Some(&json!("2")));
    for reference in [
        json!({"name": "box"}),
        json!({"name": "box2"}),
        json!({"machine_id": MACHINE}),
    ] {
        let read = get(&mut client, reference).await?;
        assert_eq!(
            read.pointer("/result/machine_id"),
            Some(&json!(MACHINE)),
            "{read}"
        );
        assert_eq!(read.pointer("/result/remote_boot_id"), Some(&json!(BOOT)));
    }
    drop(client);
    local.close().await?;
    remote.finish().await
}

#[tokio::test]
async fn a_runtime_that_proves_another_machine_is_refused_before_anything_is_recorded() -> TestResult
{
    let mut remote = Remote::start()?;
    let local = Local::start(BOUND).await?;
    let mut client = local.client(&local.bearer).await?;

    let refused = client
        .call(
            "machine.add",
            &expecting(
                add("box", &remote.relay()?, &remote.bearer),
                json!("wrong-machine"),
            ),
        )
        .await?;

    assert_eq!(kind(&refused), "trust_mismatch", "{refused}");
    assert_eq!(
        kind(&get(&mut client, json!({"name": "box"})).await?),
        "not_found"
    );
    let retried = client
        .call("machine.add", &add("box", &remote.relay()?, &remote.bearer))
        .await?;
    assert_eq!(
        retried.pointer("/result/name"),
        Some(&json!("box")),
        "{retried}"
    );
    drop(client);
    local.close().await?;
    remote.finish().await
}

#[tokio::test]
async fn a_credential_the_remote_did_not_issue_fails_authentication() -> TestResult {
    let mut remote = Remote::start()?;
    let local = Local::start(BOUND).await?;
    let mut client = local.client(&local.bearer).await?;
    let foreign = remote::bootstrap(remote::identity(MACHINE, 9)?)?.1;

    let refused = client
        .call("machine.add", &add("box", &remote.relay()?, &foreign))
        .await?;

    assert_eq!(kind(&refused), "auth_failed", "{refused}");
    drop(client);
    local.close().await?;
    remote.finish().await
}

#[tokio::test]
async fn the_same_machine_cannot_be_added_under_a_second_name() -> TestResult {
    let mut remote = Remote::start()?;
    let local = Local::start(BOUND).await?;
    let mut client = local.client(&local.bearer).await?;
    client
        .call("machine.add", &add("box", &remote.relay()?, &remote.bearer))
        .await?;

    let again = client
        .call(
            "machine.add",
            &add("other", &remote.relay()?, &remote.bearer),
        )
        .await?;
    let taken = client
        .call("machine.add", &add("box", &["false".to_owned()], "x"))
        .await?;

    assert_eq!(kind(&again), "machine_exists", "{again}");
    assert_eq!(kind(&taken), "duplicate_name", "{taken}");
    drop(client);
    local.close().await?;
    remote.finish().await
}

#[tokio::test]
async fn adds_without_trust_or_grant_or_with_a_silent_or_chatty_transport_are_typed() -> TestResult
{
    let local = Local::start(Duration::from_millis(300)).await?;
    let mut owner = local.client(&local.bearer).await?;
    let mut viewer = local.client(&local.observer()?).await?;
    let script = |text: &str| vec!["sh".to_owned(), "-c".to_owned(), text.to_owned()];

    let untrusted = owner
        .call(
            "machine.add",
            &expecting(add("a", &["false".to_owned()], "x"), Value::Null),
        )
        .await?;
    let forbidden = viewer
        .call("machine.add", &add("b", &["false".to_owned()], "x"))
        .await?;
    let silent = owner
        .call(
            "machine.add",
            &add("c", &script("exec cat >/dev/null"), "x"),
        )
        .await?;
    let banner = owner
        .call(
            "machine.add",
            &add(
                "d",
                &script("echo 'Last login: today'; exec cat >/dev/null"),
                "x",
            ),
        )
        .await?;
    let viewer_read = get(&mut viewer, json!({"name": "a"})).await?;

    assert_eq!(kind(&untrusted), "trust_required", "{untrusted}");
    assert_eq!(kind(&forbidden), "forbidden", "{forbidden}");
    assert_eq!(kind(&silent), "handshake_timeout", "{silent}");
    assert_eq!(kind(&banner), "transport_failed", "{banner}");
    assert_eq!(kind(&viewer_read), "not_found", "{viewer_read}");
    drop((owner, viewer));
    local.close().await
}
