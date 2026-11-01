use std::path::PathBuf;

use ovis_terminal_protocol::domain::boundary_policy::RelativePath;

use super::{PathError, ScopedRoot};

const FIELD: &str = "params.store";

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A private directory with `root/` (the authorized root) and `outside/secret` beside it.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self, std::io::Error> {
        let base = std::env::temp_dir().join(format!("ovis-t58-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(base.join("root").join("stores"))?;
        std::fs::create_dir_all(base.join("outside"))?;
        std::fs::write(base.join("outside").join("secret"), b"secret")?;
        std::fs::write(base.join("root").join("stores").join("a.jsonl"), b"a")?;
        Ok(Self(base))
    }

    fn root(&self) -> PathBuf {
        self.0.join("root")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            eprintln!("cannot remove {}: {error}", self.0.display());
        }
    }
}

fn relative(text: &str) -> Result<RelativePath, Box<dyn std::error::Error>> {
    Ok(RelativePath::parse(FIELD, text)?)
}

fn kind(outcome: Result<impl Sized, PathError>) -> &'static str {
    outcome.map_or_else(|error| error.kind(), |_| "ok")
}

#[test]
fn a_path_below_the_root_resolves_inside_it() -> TestResult {
    let scratch = Scratch::new()?;
    let root = ScopedRoot::open(&scratch.root())?;

    let resolved = root.resolve((FIELD, &relative("stores/a.jsonl")?))?;

    assert!(resolved.path().starts_with(root.path()));
    assert_eq!(kind(resolved.recheck()), "ok");
    Ok(())
}

#[test]
fn a_missing_path_or_root_is_reported_by_its_own_kind() -> TestResult {
    let scratch = Scratch::new()?;
    let root = ScopedRoot::open(&scratch.root())?;

    let missing = root.resolve((FIELD, &relative("stores/none")?));
    let no_root = ScopedRoot::open(&scratch.0.join("absent"));

    assert_eq!(kind(missing), "path_not_found");
    assert_eq!(kind(no_root), "store_unavailable");
    Ok(())
}

#[cfg(unix)]
fn link(target: &std::path::Path, at: &std::path::Path) -> Result<(), std::io::Error> {
    std::os::unix::fs::symlink(target, at)
}

#[cfg(unix)]
#[test]
fn a_symlink_leaving_the_root_is_forbidden_and_one_inside_is_followed() -> TestResult {
    let scratch = Scratch::new()?;
    let stores = scratch.root().join("stores");
    link(
        &scratch.0.join("outside").join("secret"),
        &stores.join("escape"),
    )?;
    link(&stores.join("a.jsonl"), &stores.join("alias"))?;
    let root = ScopedRoot::open(&scratch.root())?;

    let escape = root.resolve((FIELD, &relative("stores/escape")?));
    let alias = root.resolve((FIELD, &relative("stores/alias")?))?;

    assert_eq!(kind(escape), "forbidden");
    assert_eq!(alias.path(), root.path().join("stores").join("a.jsonl"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_symlink_retargeted_after_authorization_is_refused_at_use() -> TestResult {
    let scratch = Scratch::new()?;
    let stores = scratch.root().join("stores");
    std::fs::write(stores.join("b.jsonl"), b"b")?;
    link(&stores.join("a.jsonl"), &stores.join("current"))?;
    let root = ScopedRoot::open(&scratch.root())?;
    let authorized = root.resolve((FIELD, &relative("stores/current")?))?;

    std::fs::remove_file(stores.join("current"))?;
    link(
        &scratch.0.join("outside").join("secret"),
        &stores.join("current"),
    )?;
    let escaped = authorized.recheck();
    std::fs::remove_file(stores.join("current"))?;
    link(&stores.join("b.jsonl"), &stores.join("current"))?;
    let replaced = authorized.recheck();

    assert_eq!(kind(escaped), "forbidden");
    assert!(matches!(replaced, Err(PathError::Replaced { .. })));
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_file_swapped_under_the_same_name_is_refused_at_use() -> TestResult {
    let scratch = Scratch::new()?;
    let file = scratch.root().join("stores").join("a.jsonl");
    let root = ScopedRoot::open(&scratch.root())?;
    let authorized = root.resolve((FIELD, &relative("stores/a.jsonl")?))?;
    let held = std::fs::File::open(&file)?;

    std::fs::remove_file(&file)?;
    std::fs::write(&file, b"replacement")?;
    let swapped = authorized.recheck();
    drop(held);

    assert!(matches!(swapped, Err(PathError::Replaced { .. })));
    Ok(())
}
