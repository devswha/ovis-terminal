//! Revocations that outlive their reclaimed credential records, so a session or queued
//! decision on a reclaimed revoked credential still fails with `credential_revoked`.

use std::collections::HashMap;

use ovis_terminal_protocol::schema::scalar::TimestampMs;

use crate::identity::{CredentialEpoch, CredentialId, PrincipalId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Tombstone {
    pub(super) credential: CredentialId,
    pub(super) epoch: CredentialEpoch,
    pub(super) principal: PrincipalId,
    pub(super) expires_at: TimestampMs,
}

/// A bounded set of revoked credential ids; credential ids are never reused, so a tombstone
/// can only ever match the credential it was written for.
#[derive(Debug)]
pub(super) struct Tombstones {
    capacity: usize,
    revoked: HashMap<CredentialId, Tombstone>,
}

impl Tombstones {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            revoked: HashMap::new(),
        }
    }

    pub(super) fn get(&self, credential: CredentialId) -> Option<Tombstone> {
        self.revoked.get(&credential).cloned()
    }

    /// Records a revocation; when the set is full it first forgets revocations of credentials
    /// that have expired, and refuses (returns `false`) if every entry is still unexpired.
    pub(super) fn bury(&mut self, tombstone: Tombstone, at: TimestampMs) -> bool {
        if self.revoked.len() >= self.capacity {
            self.revoked
                .retain(|_, kept| at.get() < kept.expires_at.get());
        }
        if self.revoked.len() >= self.capacity {
            return false;
        }
        self.revoked.insert(tombstone.credential, tombstone);
        true
    }
}
