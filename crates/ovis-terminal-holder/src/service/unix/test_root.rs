//! A short owner-only directory per test, removed afterwards.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use super::{ServiceEnvironment, ServicePaths, ServiceSession, ServiceVar};

static NEXT: AtomicU32 = AtomicU32::new(0);

#[derive(Debug)]
pub(super) struct TestRoot(PathBuf);

impl TestRoot {
    /// `/tmp`, not `temp_dir()`: Darwin's per-user temp directory is too long for sockets.
    pub(super) fn new(tag: &str) -> Self {
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = PathBuf::from(format!("/tmp/ovs-{tag}-{}-{unique}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }

    pub(super) fn environment(&self) -> ServiceEnvironment {
        ServiceEnvironment::default()
            .with(ServiceVar::RuntimeDir, self.0.join("run"))
            .with(ServiceVar::Home, self.0.join("home"))
    }

    pub(super) fn paths(&self, session: &str) -> ServicePaths {
        ServicePaths::resolve(self.environment(), ServiceSession::parse(session).unwrap()).unwrap()
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap_or_default();
    }
}
