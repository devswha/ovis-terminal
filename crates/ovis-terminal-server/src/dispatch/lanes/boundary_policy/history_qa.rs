//! `agent.history` through the real dispatcher: listed below the root, refused outside it.

use std::os::unix::fs::symlink;
use std::sync::Arc;

use ovis_terminal_authz::validation::ScopedRoot;
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::OperationProducer;
use ovis_terminal_server::dispatch::lanes::boundary_policy::AgentHistory;
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};

use super::support::socket::{Client, Fixture};
use super::support::{Core, Dispatch, TestResult, at, error_kind, request, runtime};

struct Rig {
    fixture: Fixture,
    core: Arc<Core>,
    dispatch: Arc<Dispatch>,
}

fn rig() -> TestResult<Rig> {
    let fixture = Fixture::new()?;
    let (stores, outside) = (fixture.path("stores"), fixture.path("outside"));
    std::fs::create_dir_all(stores.join("claude"))?;
    std::fs::create_dir_all(&outside)?;
    std::fs::write(outside.join("secret"), b"secret")?;
    std::fs::write(stores.join("claude").join("s1.jsonl"), b"{}")?;
    std::fs::write(stores.join("claude").join("s2.jsonl"), b"{}")?;
    symlink(
        outside.join("secret"),
        stores.join("claude").join("linked.jsonl"),
    )?;
    symlink(&outside, stores.join("escape"))?;
    let identity = runtime(1)?;
    let store = Store::open(&fixture.path("db"), StoreOptions::default())?;
    let core = Arc::new(Coordinator::new(
        store,
        identity,
        OperationProducer::new(identity.runtime_id()),
    ));
    let mut dispatch = Dispatch::new(Arc::clone(&fixture.authority), identity, Arc::clone(&core));
    let root = ScopedRoot::open(&stores)?;
    dispatch.register_read(AgentHistory::new(root, identity.machine_id()))?;
    Ok(Rig {
        fixture,
        core,
        dispatch: Arc::new(dispatch),
    })
}

async fn history(client: &mut Client, (id, params): (&str, Value)) -> TestResult<Value> {
    client.send(&request(id, "agent.history", &params)).await?;
    client.expect_next().await
}

fn names(reply: &Value) -> Vec<String> {
    at(reply, "/result/sessions")
        .as_array()
        .map(|sessions| {
            sessions
                .iter()
                .filter_map(|session| at(session, "/value").as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn a_store_below_the_root_lists_its_sessions_and_skips_symlinked_entries() -> TestResult {
    let rig = rig()?;
    let server = rig.fixture.serve(1, Arc::clone(&rig.dispatch)).await?;
    let mut client = rig.fixture.connect().await?;
    client.hello("1", &rig.fixture.bearer).await?;

    let all = history(&mut client, ("all", json!({"store": "claude"}))).await?;
    let first = history(
        &mut client,
        ("first", json!({"store": "claude", "page": {"limit": 1}})),
    )
    .await?;
    let second = history(
        &mut client,
        (
            "second",
            json!({"store": "claude", "page": {"limit": 1, "after": "1"}}),
        ),
    )
    .await?;
    let root = history(&mut client, ("root", json!({}))).await?;

    let mut listed = names(&all);
    listed.sort();
    assert_eq!(listed, ["s1.jsonl", "s2.jsonl"]);
    assert_eq!(at(&all, "/result/sessions/0/source"), "claude");
    assert_eq!(at(&all, "/result/page/total"), 2);
    assert_eq!(at(&first, "/result/page/next"), "1");
    assert_eq!(at(&second, "/result/page/next"), &Value::Null);
    let mut paged = [names(&first), names(&second)].concat();
    paged.sort();
    assert_eq!(paged, listed);
    assert_eq!(at(&root, "/result/page/total"), 0);
    drop(client);
    server.shutdown().await?;
    close(rig).await
}

#[tokio::test]
async fn traversal_symlink_escapes_oversize_and_missing_stores_are_refused_by_kind() -> TestResult {
    let rig = rig()?;
    let server = rig.fixture.serve(1, Arc::clone(&rig.dispatch)).await?;
    let mut client = rig.fixture.connect().await?;
    client.hello("1", &rig.fixture.bearer).await?;

    let mut kinds = Vec::new();
    for store in [
        "../../secret".to_owned(),
        "escape".to_owned(),
        "/etc".to_owned(),
        "x".repeat(4097),
        "absent".to_owned(),
        "claude/linked.jsonl".to_owned(),
    ] {
        let reply = history(&mut client, ("h", json!({"store": store}))).await?;
        let field = at(&reply, "/error/data/field_path")
            .as_str()
            .map(str::to_owned);
        kinds.push((error_kind(&reply).to_owned(), field));
    }
    let numeric = history(&mut client, ("n", json!({"store": 7}))).await?;

    let store = Some("params.store".to_owned());
    assert_eq!(
        kinds,
        [
            ("forbidden".to_owned(), store.clone()),
            ("forbidden".to_owned(), store.clone()),
            ("forbidden".to_owned(), store.clone()),
            ("too_large".to_owned(), store.clone()),
            ("path_not_found".to_owned(), store.clone()),
            ("forbidden".to_owned(), store),
        ]
    );
    assert_eq!(error_kind(&numeric), "invalid_params");
    drop(client);
    server.shutdown().await?;
    close(rig).await
}

async fn close(rig: Rig) -> TestResult {
    rig.dispatch.shutdown().await?;
    drop(rig.dispatch);
    let core = Arc::try_unwrap(rig.core).map_err(|_| "core still owned")?;
    tokio::task::spawn_blocking(move || core.close()).await??;
    Ok(())
}
