//! An authorized root and the paths resolved below it.

use std::io;
use std::path::{Path, PathBuf};

use ovis_terminal_protocol::domain::boundary_policy::RelativePath;

use super::error::PathError;
use super::identity::FileIdentity;

/// A directory callers may name paths below; held in canonical form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedRoot {
    canonical: PathBuf,
}

/// A path that resolved inside its root, with the object it resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainedPath {
    root: PathBuf,
    requested: PathBuf,
    canonical: PathBuf,
    identity: FileIdentity,
    field: &'static str,
}

impl ScopedRoot {
    /// Resolves `root` once; every later containment check compares against this form.
    ///
    /// # Errors
    ///
    /// [`PathError::Root`] when the root does not exist or cannot be resolved.
    pub fn open(root: &Path) -> Result<Self, PathError> {
        let canonical = root.canonicalize().map_err(PathError::Root)?;
        Ok(Self { canonical })
    }

    /// The root in canonical form.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.canonical
    }

    /// The root itself as a contained path, for listing it.
    ///
    /// # Errors
    ///
    /// [`PathError::Root`] when the root no longer resolves.
    pub fn itself(&self, field: &'static str) -> Result<ContainedPath, PathError> {
        let metadata = std::fs::metadata(&self.canonical).map_err(PathError::Root)?;
        Ok(ContainedPath {
            root: self.canonical.clone(),
            requested: self.canonical.clone(),
            canonical: self.canonical.clone(),
            identity: FileIdentity::of(&metadata),
            field,
        })
    }

    /// Resolves `relative` for parameter `field`, following every symlink.
    ///
    /// # Errors
    ///
    /// [`PathError::NotFound`] when nothing exists there, [`PathError::Escaped`] when the
    /// resolved object lies outside the root and [`PathError::Io`] for other failures.
    pub fn resolve(
        &self,
        (field, relative): (&'static str, &RelativePath),
    ) -> Result<ContainedPath, PathError> {
        let requested = self.canonical.join(relative.as_path());
        let (canonical, identity) = settle(field, (&self.canonical, &requested))?;
        Ok(ContainedPath {
            root: self.canonical.clone(),
            requested,
            canonical,
            identity,
            field,
        })
    }
}

impl ContainedPath {
    /// The resolved object's canonical path, always inside the root.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.canonical
    }

    /// Repeats the containment decision at the moment of use.
    ///
    /// # Errors
    ///
    /// [`PathError::Replaced`] when the name now resolves to another path or object,
    /// [`PathError::Escaped`] when it now leaves the root, and [`PathError::NotFound`] or
    /// [`PathError::Io`] when it no longer resolves.
    pub fn recheck(&self) -> Result<(), PathError> {
        let (canonical, identity) = settle(self.field, (&self.root, &self.requested))?;
        if canonical == self.canonical && identity == self.identity {
            Ok(())
        } else {
            Err(PathError::Replaced { field: self.field })
        }
    }
}

/// Canonicalizes `requested` and requires the result to stay below `root`.
fn settle(
    field: &'static str,
    (root, requested): (&Path, &Path),
) -> Result<(PathBuf, FileIdentity), PathError> {
    let canonical = requested.canonicalize().map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            PathError::NotFound { field }
        } else {
            PathError::Io { field, source }
        }
    })?;
    if !canonical.starts_with(root) {
        return Err(PathError::Escaped { field });
    }
    let metadata =
        std::fs::metadata(&canonical).map_err(|source| PathError::Io { field, source })?;
    Ok((canonical, FileIdentity::of(&metadata)))
}
