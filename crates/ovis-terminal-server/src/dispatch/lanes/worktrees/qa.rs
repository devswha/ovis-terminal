//! The worktree lane over the real socket.
//!
//! Explicit identities, enumerated refusals that preserve content, and committed partial
//! effects reported on cancellation.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::support::socket::Client;
use super::support::{TestResult, at, error_kind, request};
use super::worktree_fixture::{Fixture, git, signalled};

async fn ask(client: &mut Client, (id, method): (&str, &str), params: &Value) -> TestResult<Value> {
    client.send(&request(id, method, params)).await?;
    let reply = client.expect_next().await?;
    assert_eq!(at(&reply, "/id"), id);
    Ok(reply)
}

async fn create(client: &mut Client, (op, branch): (&str, &str)) -> TestResult<Value> {
    let primary = ask(
        client,
        ("list", "worktree.list"),
        &json!({"repository_id": "repo:main"}),
    )
    .await?;
    let source = at(&primary, "/result/worktrees/0/checkout_id").clone();
    let params =
        json!({"op_id": op, "repository_id": "repo:main", "source": source, "branch": branch});
    let created = ask(client, ("create", "worktree.create"), &params).await?;
    assert_eq!(
        at(&created, "/result/worktree/source"),
        &source,
        "{created}"
    );
    Ok(at(&created, "/result").clone())
}

#[tokio::test(flavor = "multi_thread")]
async fn happy_create_open_and_enumerated_group_removal() -> TestResult {
    let fixture = Fixture::new(PathBuf::from("git"))?;
    let (server, mut client) = fixture.connect().await?;
    let [op51, op52, op53, op54] = [(); 4].map(|()| uuid::Uuid::now_v7().to_string());
    let created = create(&mut client, (&op51, "feature/a")).await?;
    let checkout = at(&created, "/worktree/checkout_id").clone();
    assert!(
        at(&created, "/seq")
            .as_str()
            .is_some_and(|seq| seq.parse::<u64>().is_ok())
    );
    let open = json!({"op_id": &op52, "checkout_id": checkout});
    let opened = ask(&mut client, ("open", "worktree.open"), &open).await?;
    let workspace = at(&opened, "/result/affected_workspaces/0").clone();
    assert_eq!(
        at(&opened, "/result/worktree/workspaces"),
        &json!([workspace])
    );
    let refuse = json!({"op_id": &op53, "checkout_id": checkout,
        "scope": {"group": false, "force": false}});
    let refused = ask(&mut client, ("narrow", "worktree.remove"), &refuse).await?;
    assert_eq!(error_kind(&refused), "scope_required");
    assert_eq!(
        at(&refused, "/error/data/affected"),
        &json!([checkout, workspace])
    );
    assert_eq!(at(&refused, "/error/data/op_id"), op53.as_str());
    let path = at(&created, "/worktree/path")
        .as_str()
        .ok_or("path")?
        .to_owned();
    assert!(Path::new(&path).is_dir());
    let state = ask(&mut client, ("op", "op.get"), &json!({"op_id": op53})).await?;
    assert_eq!(at(&state, "/result/state"), "failed");
    let group = json!({"op_id": &op54, "checkout_id": checkout,
        "scope": {"group": true, "force": false}});
    let removed = ask(&mut client, ("group", "worktree.remove"), &group).await?;
    assert_eq!(
        at(&removed, "/result/affected_workspaces"),
        &json!([workspace])
    );
    assert!(!Path::new(&path).exists());
    assert_eq!(
        fixture
            .workspaces
            .closed
            .lock()
            .map_err(|_| "poisoned")?
            .len(),
        1
    );
    let listed = ask(
        &mut client,
        ("after", "worktree.list"),
        &json!({"repository_id": "repo:main"}),
    )
    .await?;
    assert_eq!(
        at(&listed, "/result/worktrees").as_array().map(Vec::len),
        Some(1)
    );
    drop(client);
    fixture.close(server).await
}

#[tokio::test(flavor = "multi_thread")]
async fn failure_dirty_checkout_is_refused_with_its_identity_and_kept() -> TestResult {
    let fixture = Fixture::new(PathBuf::from("git"))?;
    let (server, mut client) = fixture.connect().await?;
    let [op55, op56] = [(); 2].map(|()| uuid::Uuid::now_v7().to_string());
    let created = create(&mut client, (&op55, "dirty")).await?;
    let checkout = at(&created, "/worktree/checkout_id").clone();
    let note = Path::new(at(&created, "/worktree/path").as_str().ok_or("path")?).join("notes.txt");
    std::fs::write(&note, "unsaved")?;
    let flat = json!({"checkout_id": checkout, "force": false, "group": false});
    let rejected = ask(&mut client, ("flat", "worktree.remove"), &flat).await?;
    assert_eq!(error_kind(&rejected), "invalid_params", "{rejected}");
    let remove = json!({"op_id": &op56, "checkout_id": checkout,
        "scope": {"group": false, "force": false}});
    let refused = ask(&mut client, ("dirty", "worktree.remove"), &remove).await?;
    assert_eq!(error_kind(&refused), "checkout_dirty");
    assert_eq!(at(&refused, "/error/data/affected"), &json!([checkout]));
    assert_eq!(std::fs::read_to_string(&note)?, "unsaved");
    let listed = ask(
        &mut client,
        ("list", "worktree.list"),
        &json!({"repository_id": "repo:main"}),
    )
    .await?;
    assert_eq!(at(&listed, "/result/worktrees/1/dirty"), true);
    assert!(
        fixture
            .workspaces
            .closed
            .lock()
            .map_err(|_| "poisoned")?
            .is_empty()
    );
    drop(client);
    fixture.close(server).await
}

#[tokio::test(flavor = "multi_thread")]
async fn op_cancel_kills_a_hung_create_and_reports_what_committed() -> TestResult {
    let scratch = std::env::temp_dir().join(format!("ovt45-srv-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir(&scratch)?;
    let (ready, hold, script) = (
        scratch.join("ready"),
        scratch.join("hold"),
        scratch.join("git.sh"),
    );
    for fifo in [&ready, &hold] {
        assert!(
            std::process::Command::new("mkfifo")
                .arg(fifo)
                .status()?
                .success()
        );
    }
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \" $* \" in\n*\" worktree add \"*) git \"$@\" || exit $?\n\
         echo ready > '{}'\nread held < '{}' ;;\n*) exec git \"$@\" ;;\nesac\n",
            ready.display(),
            hold.display()
        ),
    )?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    let fixture = Fixture::new(script)?;
    let (server, mut client) = fixture.connect().await?;
    let listed = ask(
        &mut client,
        ("list", "worktree.list"),
        &json!({"repository_id": "repo:main"}),
    )
    .await?;
    let source = at(&listed, "/result/worktrees/0/checkout_id").clone();
    let op = uuid::Uuid::now_v7().to_string();
    let op = op.as_str();
    let params =
        json!({"op_id": op, "repository_id": "repo:main", "source": source, "branch": "partial"});
    client
        .send(&request("slow", "worktree.create", &params))
        .await?;
    let line = signalled(ready).await;
    client
        .send(&request("cancel", "op.cancel", &json!({"op_id": op})))
        .await?;
    assert_eq!(line?.trim(), "ready");
    let (first, second) = (client.expect_next().await?, client.expect_next().await?);
    let slow = if at(&first, "/id") == "slow" {
        first
    } else {
        second
    };
    assert_eq!(error_kind(&slow), "cancelled", "{slow}");
    let committed = at(&slow, "/error/data/affected");
    assert_eq!(at(committed, "/0"), "branch:partial");
    let after = ask(
        &mut client,
        ("after", "worktree.list"),
        &json!({"repository_id": "repo:main"}),
    )
    .await?;
    assert_eq!(
        at(committed, "/1"),
        at(&after, "/result/worktrees/1/checkout_id")
    );
    assert!(
        git(
            &fixture.repo,
            &["rev-parse", "--verify", "refs/heads/partial"]
        )
        .is_ok()
    );
    drop(client);
    fixture.close(server).await?;
    std::fs::remove_dir_all(scratch)?;
    Ok(())
}
