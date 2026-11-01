//! Context that places a refusal relative to the caller's authority or a remote machine.

use super::BoundaryError;
use crate::schema::records::LeaseRecord;
use crate::schema::scalar::{EventSeq, PrincipalEpoch};

impl BoundaryError {
    /// Adds the principal's current authority epoch to a credential refusal.
    #[must_use]
    pub fn with_principal_epoch(mut self, epoch: PrincipalEpoch) -> Self {
        self.data_mut().principal_epoch = Some(epoch);
        self
    }

    /// Adds the last remote sequence mirrored from the machine a call was routed to.
    #[must_use]
    pub fn with_last_seen_seq(mut self, seq: EventSeq) -> Self {
        self.data_mut().last_seen_seq = Some(seq);
        self
    }

    /// Adds the lease that holds the slot now to a refusal that named a stale epoch.
    #[must_use]
    pub fn with_current_lease(mut self, current: LeaseRecord) -> Self {
        self.data_mut().current = Some(Box::new(current));
        self
    }
}
