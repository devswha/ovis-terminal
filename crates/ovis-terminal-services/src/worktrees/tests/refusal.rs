use std::path::Path;

use ovis_terminal_protocol::schema::methods::worktree::WorktreeCreateParams;
use serde_json::json;

use super::fixture::{Fixture, TestResult, git, nth, params, targets};

#[tokio::test]
async fn dirty_content_survives_until_force_is_given() -> TestResult {
    let fixture = Fixture::new()?;
    let created = fixture.create("dirty").await?;
    let note = Path::new(&created.path).join("notes.txt");
    std::fs::write(&note, "unsaved work")?;
    assert!(nth(&fixture.list().await?, 1)?.dirty);
    for scope in [(false, false), (true, false)] {
        let refused = fixture.remove(&created.checkout_id, scope).await?;
        let refused = refused
            .err()
            .ok_or("dirty removal without force succeeded")?;
        assert_eq!(
            refused.kind(),
            "checkout_dirty",
            "group scope is not force: {scope:?}"
        );
        assert_eq!(targets(&refused), [created.checkout_id.as_str()]);
        assert_eq!(std::fs::read_to_string(&note)?, "unsaved work");
    }
    fixture
        .remove(&created.checkout_id, (false, true))
        .await??;
    assert!(!note.exists());
    Ok(())
}

#[tokio::test]
async fn a_locked_checkout_needs_force_and_git_needs_it_twice() -> TestResult {
    let fixture = Fixture::new()?;
    let created = fixture.create("locked").await?;
    git(&fixture.repo, &["worktree", "lock", &created.path])?;
    assert!(nth(&fixture.list().await?, 1)?.locked);
    let refused = fixture.remove(&created.checkout_id, (true, false)).await?;
    assert_eq!(
        refused.err().map(|error| error.kind()),
        Some("checkout_locked")
    );
    assert!(Path::new(&created.path).is_dir());
    fixture
        .remove(&created.checkout_id, (false, true))
        .await??;
    assert!(!Path::new(&created.path).exists());
    Ok(())
}

#[tokio::test]
async fn misnamed_or_unknown_targets_are_refused_before_git_changes_anything() -> TestResult {
    let fixture = Fixture::new()?;
    let primary = fixture.primary().await?;
    let refused = fixture.remove(&primary, (true, true)).await?;
    assert_eq!(
        refused.err().map(|error| error.kind()),
        Some("path_forbidden")
    );
    for (branch, kind) in [
        ("-x", "invalid_name"),
        ("a..b", "invalid_name"),
        ("main", "duplicate_name"),
    ] {
        let refused = fixture
            .create(branch)
            .await
            .err()
            .ok_or("refused branch was created")?;
        let refused = refused
            .downcast_ref::<crate::worktrees::WorktreeError>()
            .ok_or("untyped")?;
        assert_eq!(refused.kind(), kind, "{branch}");
    }
    let other: WorktreeCreateParams = params(json!({"op_id": "op", "repository_id": "repo:other",
        "source": primary, "branch": "x"}))?;
    let refused = fixture.service.create(fixture.call(&other)).await;
    assert_eq!(refused.err().map(|error| error.kind()), Some("not_found"));
    let unknown = params(json!({"checkout_id": "checkout:none", "op_id": "op",
        "scope": {"group": true, "force": true}}))?;
    let refused = fixture.service.remove(fixture.call(&unknown)).await;
    assert_eq!(refused.err().map(|error| error.kind()), Some("not_found"));
    assert_eq!(fixture.list().await?.len(), 1);
    assert_eq!(git(&fixture.repo, &["branch", "--list", "x"])?, "");
    Ok(())
}
