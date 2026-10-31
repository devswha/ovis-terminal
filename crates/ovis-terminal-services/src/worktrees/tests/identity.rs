use std::path::Path;

use ovis_terminal_protocol::schema::methods::worktree::WorktreeOpenParams;
use serde_json::json;

use super::fixture::{Fixture, TestResult, git, nth, params};

#[tokio::test]
async fn a_checkout_recreated_at_the_same_path_never_inherits_the_old_identity() -> TestResult {
    // Given a checkout created here, so it carries a source relationship.
    let fixture = Fixture::new()?;
    let old = fixture.create("first").await?;
    assert!(old.source.is_some());
    // When it is removed and a different checkout is added at the same path, unobserved.
    git(&fixture.repo, &["worktree", "remove", &old.path])?;
    git(
        &fixture.repo,
        &["worktree", "add", "-q", "-b", "second", &old.path, "HEAD"],
    )?;
    // Then the new checkout has a new identity, no source, and the old identity is gone.
    let listed = fixture.list().await?;
    let new = nth(&listed, 1)?;
    assert_eq!(new.path, old.path);
    assert_eq!(new.branch.as_deref(), Some("second"));
    assert_ne!(new.checkout_id, old.checkout_id);
    assert_eq!(
        new.source, None,
        "the old source relationship is not inherited"
    );
    let removed = fixture.remove(&old.checkout_id, (true, true)).await?;
    assert_eq!(removed.err().map(|error| error.kind()), Some("not_found"));
    assert!(
        Path::new(&old.path).is_dir(),
        "the stale identity removed nothing"
    );
    let open: WorktreeOpenParams = params(json!({"op_id": "op", "checkout_id": old.checkout_id}))?;
    let reopened = fixture.service.open(fixture.call(&open)).await;
    assert_eq!(reopened.err().map(|error| error.kind()), Some("not_found"));
    Ok(())
}

#[tokio::test]
async fn a_moved_checkout_keeps_its_identity_and_source() -> TestResult {
    let fixture = Fixture::new()?;
    let created = fixture.create("moving").await?;
    let moved = fixture.root.join("checkouts").join("moved");
    git(
        &fixture.repo,
        &[
            "worktree",
            "move",
            &created.path,
            &moved.display().to_string(),
        ],
    )?;
    let listed = fixture.list().await?;
    let seen = nth(&listed, 1)?;
    assert_eq!(seen.checkout_id, created.checkout_id);
    assert_eq!(seen.source, created.source);
    assert_eq!(Path::new(&seen.path), moved);
    Ok(())
}
