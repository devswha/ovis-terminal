use std::path::Path;

use ovis_terminal_protocol::schema::methods::worktree::WorktreeOpenParams;
use serde_json::json;

use super::fixture::{Fixture, TestResult, git, nth, params, targets};

#[tokio::test]
async fn create_open_and_group_removal_report_every_identity() -> TestResult {
    // Given a repository whose primary checkout is the only one.
    let fixture = Fixture::new()?;
    let primary = fixture.list().await?;
    assert_eq!(primary.len(), 1);
    assert_eq!(nth(&primary, 0)?.source, None);
    assert_eq!(nth(&primary, 0)?.branch.as_deref(), Some("main"));
    let primary = nth(&primary, 0)?.checkout_id.clone();
    // When a checkout is created from it and a workspace is opened on it.
    let created = fixture.create("feature/a").await?;
    assert_eq!(created.source.as_ref(), Some(&primary));
    assert_eq!(created.branch.as_deref(), Some("feature/a"));
    assert!(!created.dirty && !created.locked && created.workspaces.is_empty());
    assert!(Path::new(&created.path).starts_with(&fixture.root));
    assert!(
        git(
            &fixture.repo,
            &["rev-parse", "--verify", "refs/heads/feature/a"]
        )
        .is_ok()
    );
    let listed = fixture.list().await?;
    assert_eq!(listed.len(), 2);
    assert_eq!(
        nth(&listed, 1)?,
        &created,
        "list reports the source relationship it was created with"
    );
    let open: WorktreeOpenParams =
        params(json!({"op_id": "op", "checkout_id": created.checkout_id}))?;
    let opened = fixture.service.open(fixture.call(&open)).await?;
    assert_eq!(opened.workspaces.len(), 1);
    let workspace = *nth(&opened.workspaces, 0)?;
    assert_eq!(
        nth(&opened.record.workspaces, 0)?.as_str(),
        workspace.to_string()
    );
    assert!(opened.record.revision > created.revision);
    // Then removal without group scope names the checkout and its workspace and keeps both.
    let refused = fixture.remove(&created.checkout_id, (false, false)).await?;
    let refused = refused
        .err()
        .ok_or("removal without group scope succeeded")?;
    assert_eq!(refused.kind(), "scope_required");
    assert_eq!(
        targets(&refused),
        [
            created.checkout_id.as_str().to_owned(),
            workspace.to_string()
        ]
    );
    assert!(Path::new(&created.path).is_dir());
    assert!(
        fixture
            .workspaces
            .closed
            .lock()
            .map_err(|_| "poisoned")?
            .is_empty()
    );
    // And group scope closes exactly the enumerated workspace, then removes the checkout.
    let removed = fixture
        .remove(&created.checkout_id, (true, false))
        .await??;
    assert_eq!(removed.workspaces, [workspace]);
    assert_eq!(
        *fixture.workspaces.closed.lock().map_err(|_| "poisoned")?,
        [workspace]
    );
    assert!(!Path::new(&created.path).exists());
    let listed = fixture.list().await?;
    assert_eq!(listed.len(), 1);
    assert_eq!(nth(&listed, 0)?.checkout_id, primary);
    Ok(())
}

#[tokio::test]
async fn a_checkout_removed_outside_the_service_retires_its_identity() -> TestResult {
    let fixture = Fixture::new()?;
    let created = fixture.create("gone").await?;
    git(&fixture.repo, &["worktree", "remove", &created.path])?;
    assert_eq!(fixture.list().await?.len(), 1);
    let stale = fixture.remove(&created.checkout_id, (true, true)).await?;
    assert_eq!(stale.err().map(|error| error.kind()), Some("not_found"));
    let open: WorktreeOpenParams =
        params(json!({"op_id": "op", "checkout_id": created.checkout_id}))?;
    let reopened = fixture.service.open(fixture.call(&open)).await;
    assert_eq!(reopened.err().map(|error| error.kind()), Some("not_found"));
    assert!(
        fixture
            .workspaces
            .opened
            .lock()
            .map_err(|_| "poisoned")?
            .is_empty()
    );
    Ok(())
}
