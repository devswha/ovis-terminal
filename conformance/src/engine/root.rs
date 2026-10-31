//! A fresh owner-only directory per case, removed when the case ends.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::error::RunnerError;

static NEXT: AtomicU32 = AtomicU32::new(0);

#[derive(Debug)]
pub(crate) struct IsolatedRoot {
    path: PathBuf,
}

impl IsolatedRoot {
    /// Creates `/tmp/ovc-<pid>-<n>` with mode 0700. `/tmp`, not `temp_dir()`: Darwin's
    /// per-user temp directory pushes `<root>/run/s.sock` past the 104-byte socket limit.
    pub(crate) fn create() -> Result<Self, RunnerError> {
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let base = if cfg!(unix) {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let path = base.join(format!("ovc-{}-{unique}", std::process::id()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&path).map_err(|source| RunnerError::Io {
            operation: "create the isolated root",
            source,
        })?;
        Ok(Self { path })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for IsolatedRoot {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.path) {
            let reported = writeln!(
                std::io::stderr().lock(),
                "conformance: could not remove {}: {error}",
                self.path.display()
            );
            drop(reported);
        }
    }
}
