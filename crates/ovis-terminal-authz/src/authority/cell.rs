//! The revocation cause of one credential, shared by its record and by every session and
//! decision made under it, so reclaiming the record or its tombstone cannot erase it.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::identity::CredentialEpoch;

/// Holds 0 while the credential is active, else the epoch its revocation produced (never 0).
#[derive(Debug, Default)]
pub(super) struct RevocationCell(AtomicU64);

impl RevocationCell {
    /// Revocation runs under the table's write lock; `Release` publishes the epoch to
    /// lock-free readers that load it with `Acquire`.
    pub(super) fn revoke(&self, epoch: CredentialEpoch) {
        self.0.store(epoch.get(), Ordering::Release);
    }

    pub(super) fn revoked_at(&self) -> Option<CredentialEpoch> {
        match self.0.load(Ordering::Acquire) {
            0 => None,
            epoch => Some(CredentialEpoch::from_raw(epoch)),
        }
    }
}
