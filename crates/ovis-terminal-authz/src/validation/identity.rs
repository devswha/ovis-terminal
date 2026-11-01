//! What a resolved path is, beyond its name: the object the name reached.

use std::fs::Metadata;

/// The filesystem object a path resolved to.
///
/// On Unix the device and inode identify the object, so swapping a same-named file for
/// another one is seen even when the canonical name is unchanged. Elsewhere the standard
/// library exposes no stable file index, and the canonical name alone identifies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FileIdentity(Option<(u64, u64)>);

impl FileIdentity {
    #[cfg(unix)]
    pub(super) fn of(metadata: &Metadata) -> Self {
        use std::os::unix::fs::MetadataExt as _;
        Self(Some((metadata.dev(), metadata.ino())))
    }

    #[cfg(not(unix))]
    pub(super) const fn of(_: &Metadata) -> Self {
        Self(None)
    }
}
