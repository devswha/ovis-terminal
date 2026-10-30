//! Shared test fixtures: a private store root per test and one-key document mutations.

#![expect(
    clippy::redundant_pub_crate,
    reason = "rustc's unreachable_pub (workspace lint) requires pub(crate) here, which this nursery lint contradicts"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use ovis_terminal_store::{CasWrite, EventKind, Mutation, NewEvent, OpId, Payload, ResourceKey};

pub(crate) type Fallible<T = ()> = Result<T, Box<dyn std::error::Error>>;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

pub(crate) struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    pub(crate) fn new(label: &str) -> Fallible<Self> {
        let serial = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let name = format!("ovis-store-{label}-{}-{serial}", std::process::id());
        let path = std::env::temp_dir().join(name);
        if path.exists() {
            // A previous process with the same pid left this directory behind.
            fs::remove_dir_all(&path)?;
        }
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempRoot {
    #[expect(
        clippy::print_stderr,
        reason = "Drop cannot return the cleanup error; the test log is its only outlet"
    )]
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            eprintln!("cannot remove test root {}: {error}", self.path.display());
        }
    }
}

pub(crate) fn key(id: &str) -> ResourceKey {
    ResourceKey::new("doc", id)
}

/// A mutation whose receipt result and `doc.changed` event payload are the operation id.
pub(crate) fn set(op: &str, write: CasWrite) -> Mutation {
    let event = NewEvent::new(EventKind::new("doc.changed"), Payload::from(op));
    Mutation::new(OpId::new(op), Payload::from(op), event).compare_and_swap(write)
}
