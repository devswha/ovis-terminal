use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use ovis_terminal_protocol::schema::methods::worktree::WorktreeCreateParams;
use serde_json::json;

use super::fixture::{Fixture, TestResult, git, nth, params, signalled, targets};

/// A Git that performs `worktree add`, reports it on a FIFO, then hangs until killed.
fn hanging_git(dir: &std::path::Path) -> TestResult<(PathBuf, PathBuf)> {
    let ready = dir.join("ready.fifo");
    let hold = dir.join("hold.fifo");
    for fifo in [&ready, &hold] {
        assert!(Command::new("mkfifo").arg(fifo).status()?.success());
    }
    let script = dir.join("git.sh");
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
    Ok((script, ready))
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_hung_create_reports_the_effects_it_committed() -> TestResult {
    let scratch = std::env::temp_dir().join(format!("ovt45-git-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir(&scratch)?;
    let (script, ready) = hanging_git(&scratch)?;
    let fixture = Fixture::with_git(script, Duration::from_secs(60))?;
    let primary = fixture.primary().await?;
    let create: WorktreeCreateParams = params(json!({"op_id": "op", "repository_id": "repo:main",
        "source": primary, "branch": "partial"}))?;
    let created = fixture.service.create(fixture.call(&create));
    let reported = async {
        let line = signalled(ready).await;
        // Cancel even when the signal failed, so the hung create ends and the join returns.
        fixture.cancel.send_replace(true);
        line
    };
    let (created, reported) = tokio::time::timeout(Duration::from_secs(60), async {
        tokio::join!(created, reported)
    })
    .await?;
    assert_eq!(reported?.trim(), "ready");
    let interrupted = created.err().ok_or("cancelled create reported success")?;
    assert_eq!(interrupted.kind(), "cancelled");
    let committed = targets(&interrupted);
    assert_eq!(nth(&committed, 0)?, "branch:partial");
    assert_eq!(
        committed.len(),
        2,
        "branch and checkout both committed: {committed:?}"
    );
    fixture.cancel.send_replace(false);
    let listed = fixture.list().await?;
    assert_eq!(nth(&listed, 1)?.checkout_id.as_str(), nth(&committed, 1)?);
    assert_eq!(nth(&listed, 1)?.source.as_ref(), Some(&primary));
    assert!(
        git(
            &fixture.repo,
            &["rev-parse", "--verify", "refs/heads/partial"]
        )
        .is_ok()
    );
    drop(fixture);
    std::fs::remove_dir_all(scratch)?;
    Ok(())
}

#[tokio::test]
async fn a_git_that_never_answers_is_killed_at_its_bound() -> TestResult {
    let scratch = std::env::temp_dir().join(format!("ovt45-git-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir(&scratch)?;
    assert!(
        Command::new("mkfifo")
            .arg(scratch.join("never.fifo"))
            .status()?
            .success()
    );
    let script = scratch.join("git.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nread held < '{}'\n",
            scratch.join("never.fifo").display()
        ),
    )?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    let fixture = Fixture::with_git(script, Duration::from_millis(300))?;
    let listed = fixture.list().await;
    let error = listed.err().ok_or("a hung git answered")?;
    let error = error
        .downcast_ref::<crate::worktrees::WorktreeError>()
        .ok_or("untyped")?;
    assert_eq!((error.kind(), error.retryable()), ("vcs_failed", true));
    drop(fixture);
    std::fs::remove_dir_all(scratch)?;
    Ok(())
}
