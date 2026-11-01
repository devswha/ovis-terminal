//! Lexically contained relative paths: the one component check every root-scoped path
//! (provider stores, update archive entries) goes through before it touches a filesystem.

use std::path::{Component, Path, PathBuf};

use super::violation::{BoundaryViolation, Malformation, PayloadLimit};

/// A non-empty relative path made only of normal components.
///
/// It cannot climb (`..`), restart at a root, carry a drive prefix or hold a NUL byte, and
/// `\` is refused on every platform so a path checked on Unix cannot climb on Windows.
/// Lexical containment is necessary but not sufficient: a symlink below the root can still
/// leave it, which the filesystem-level check of the authorizing crate decides.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RelativePath(PathBuf);

impl RelativePath {
    /// Parses `text` for parameter `field`.
    ///
    /// # Errors
    ///
    /// [`BoundaryViolation::Malformed`] for an empty path or a NUL byte,
    /// [`BoundaryViolation::TooLarge`] past the path limit and
    /// [`BoundaryViolation::Traversal`] for any non-normal component or `\`.
    pub fn parse(field: &'static str, text: &str) -> Result<Self, BoundaryViolation> {
        PayloadLimit::PathBytes.check(field, text.len())?;
        if text.contains('\0') {
            return Err(BoundaryViolation::Malformed {
                field,
                reason: Malformation::NulByte,
            });
        }
        if text.contains('\\') {
            return Err(BoundaryViolation::Traversal { field });
        }
        Self::of_path(field, Path::new(text))
    }

    /// Checks an already decoded path, such as an archive entry name.
    ///
    /// # Errors
    ///
    /// [`BoundaryViolation::Malformed`] for an empty path and
    /// [`BoundaryViolation::Traversal`] for any non-normal component.
    pub fn of_path(field: &'static str, path: &Path) -> Result<Self, BoundaryViolation> {
        if path.as_os_str().is_empty() {
            return Err(BoundaryViolation::Malformed {
                field,
                reason: Malformation::EmptyPath,
            });
        }
        let normal = path
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
        if !normal {
            return Err(BoundaryViolation::Traversal { field });
        }
        Ok(Self(path.to_path_buf()))
    }

    /// The contained path, to be joined onto its authorized root.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// An opaque provider-store identity: a relative path below the authorized history root.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StoreRef {
    text: String,
    path: RelativePath,
}

impl StoreRef {
    /// The parameter a store reference arrives in.
    pub const FIELD: &'static str = "params.store";

    /// Parses the `store` parameter.
    ///
    /// # Errors
    ///
    /// The errors of [`RelativePath::parse`] for [`StoreRef::FIELD`].
    pub fn parse(text: &str) -> Result<Self, BoundaryViolation> {
        Ok(Self {
            path: RelativePath::parse(Self::FIELD, text)?,
            text: text.to_owned(),
        })
    }

    /// The reference as the caller wrote it, reported as provenance.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The contained relative path.
    #[must_use]
    pub const fn path(&self) -> &RelativePath {
        &self.path
    }
}
