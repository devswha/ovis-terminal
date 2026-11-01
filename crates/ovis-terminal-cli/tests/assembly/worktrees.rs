//! `worktree.*` on the real binary over a registered Git repository: a checkout created
//! from the primary checkout is listed, opening it adds a layout workspace, removing it
//! without group scope is refused naming that workspace, and with group scope the
//! workspace closes and only the primary checkout remains.

use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use super::support::{Served, TestResult, fresh_root, op_id};

fn git(dir: &Path, args: &[&str]) -> TestResult<()> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin");
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!("git {args:?}: {output:?}").into());
    }
    Ok(())
}

fn call(served: &Served, method: &str, params: &Value) -> TestResult<Value> {
    served.ok(&["call", method, "--params", &params.to_string()])
}

/// A root whose state registers `repo:main`, a repository with one commit.
fn registered() -> TestResult<std::path::PathBuf> {
    let root = fresh_root()?;
    let tree = root.join("wt");
    let repo = tree.join("repo");
    std::fs::create_dir_all(&repo)?;
    std::fs::create_dir_all(tree.join("checkouts"))?;
    git(&repo, &["init", "-q", "-b", "main"])?;
    std::fs::write(repo.join("README"), "kept\n")?;
    git(&repo, &["add", "README"])?;
    git(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@e",
            "commit",
            "-qm",
            "init",
        ],
    )?;
    std::os::unix::fs::DirBuilderExt::mode(&mut std::fs::DirBuilder::new(), 0o700)
        .create(root.join("state"))?;
    let registered = json!({"root": tree, "repositories": [
        {"id": "repo:main", "path": repo, "checkouts": tree.join("checkouts")}]});
    std::fs::write(
        root.join("state").join("worktrees.json"),
        registered.to_string(),
    )?;
    Ok(root)
}

fn checkouts(served: &Served) -> TestResult<Vec<Value>> {
    let listed = call(
        served,
        "worktree.list",
        &json!({"repository_id": "repo:main"}),
    )?;
    Ok(listed
        .get("worktrees")
        .and_then(Value::as_array)
        .cloned()
        .ok_or("worktrees")?)
}

#[test]
fn a_checkout_opens_a_workspace_and_its_removal_closes_exactly_that_workspace() -> TestResult {
    let served = Served::start_at(&registered()?, &[])?;
    let primary = checkouts(&served)?
        .first()
        .and_then(|checkout| checkout.get("checkout_id").cloned())
        .ok_or("primary checkout")?;
    let created = call(
        &served,
        "worktree.create",
        &json!({"op_id": op_id()?, "repository_id": "repo:main", "source": primary,
            "branch": "feature"}),
    )?;
    let checkout = created
        .pointer("/worktree/checkout_id")
        .cloned()
        .ok_or("checkout")?;
    assert_eq!(
        created.pointer("/worktree/branch"),
        Some(&json!("feature")),
        "{created}"
    );
    assert_eq!(checkouts(&served)?.len(), 2);

    let opened = call(
        &served,
        "worktree.open",
        &json!({"op_id": op_id()?, "checkout_id": checkout}),
    )?;
    let workspace = opened
        .pointer("/worktree/workspaces/0")
        .cloned()
        .ok_or("workspace")?;
    let layout = call(&served, "layout.get", &json!({}))?;
    assert_eq!(
        layout.pointer("/workspaces/0/workspace_id"),
        Some(&workspace),
        "{layout}"
    );

    let refused = served.refused(
        &[
            "call",
            "worktree.remove",
            "--params",
            &json!({"op_id": op_id()?, "checkout_id": checkout,
                "scope": {"group": false, "force": false}})
            .to_string(),
        ],
        1,
    )?;
    assert_eq!(
        refused.get("kind"),
        Some(&json!("scope_required")),
        "{refused}"
    );
    let removed = call(
        &served,
        "worktree.remove",
        &json!({"op_id": op_id()?, "checkout_id": checkout, "scope": {"group": true, "force": false}}),
    )?;
    assert_eq!(
        removed.get("affected_workspaces"),
        Some(&json!([workspace])),
        "{removed}"
    );
    let layout = call(&served, "layout.get", &json!({}))?;
    assert_eq!(layout.get("workspaces"), Some(&json!([])), "{layout}");
    assert_eq!(checkouts(&served)?.len(), 1);
    served.stop()?;
    Ok(())
}
