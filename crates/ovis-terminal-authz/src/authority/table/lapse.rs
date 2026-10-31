//! Whether a claim still holds, and the refusal that names why not.

use std::sync::Arc;

use ovis_terminal_protocol::schema::scalar::TimestampMs;

use super::{Claim, CredentialRecord, CredentialTable, Standing};
use crate::error::AuthzError;
use crate::family::IdentityFamily;
use crate::identity::{CredentialEpoch, CredentialId};

/// Why a claim no longer holds.
#[derive(Debug, Clone, Copy)]
enum Lapse {
    Revoked(CredentialEpoch),
    Expired(TimestampMs),
}

impl Claim {
    /// The bound cause, found without consulting the table, so reclaiming the record or its
    /// tombstone cannot turn a revoked or expired claim into an unknown one.
    fn lapse(&self, at: TimestampMs) -> Option<Lapse> {
        if let Some(epoch) = self.revocation.revoked_at() {
            return Some(Lapse::Revoked(epoch));
        }
        (at.get() >= self.expires_at.get()).then_some(Lapse::Expired(self.expires_at))
    }
}

impl<F: IdentityFamily> CredentialRecord<F> {
    /// Why the credential no longer holds at the claimed epoch at `at`, if it does not.
    const fn lapse(&self, claim: &Claim, at: TimestampMs) -> Option<Lapse> {
        if matches!(self.standing, Standing::Revoked) || self.epoch.get() != claim.epoch.get() {
            return Some(Lapse::Revoked(self.epoch));
        }
        if at.get() >= self.expires_at.get() {
            return Some(Lapse::Expired(self.expires_at));
        }
        None
    }
}

impl<F: IdentityFamily> CredentialTable<F> {
    /// Returns the record behind `claim` if it is still valid at `at`; a claim on a
    /// reclaimed revoked credential still fails as revoked.
    pub(in crate::authority) fn check_claim(
        &self,
        claim: &Claim,
        at: TimestampMs,
    ) -> Result<&CredentialRecord<F>, AuthzError> {
        if let Some(lapse) = claim.lapse(at) {
            return Err(self.refusal(claim, lapse));
        }
        let Some(record) = self.records.get(&claim.credential) else {
            return Err(self
                .tombstones
                .get(claim.credential)
                .map_or(AuthzError::InvalidCredential, |tombstone| {
                    self.refusal(claim, Lapse::Revoked(tombstone.epoch))
                }));
        };
        record
            .lapse(claim, at)
            .map_or(Ok(record), |lapse| Err(self.refusal(claim, lapse)))
    }

    /// The record of `credential` if it is active and unexpired at `at`.
    pub(in crate::authority) fn live(
        &self,
        credential: CredentialId,
        at: TimestampMs,
    ) -> Result<&CredentialRecord<F>, AuthzError> {
        let Some(record) = self.records.get(&credential) else {
            let tombstone = self
                .tombstones
                .get(credential)
                .ok_or(AuthzError::CredentialNotFound { credential })?;
            let claim = Claim {
                credential,
                epoch: tombstone.epoch,
                principal: tombstone.principal,
                expires_at: tombstone.expires_at,
                revocation: Arc::default(),
            };
            return Err(self.refusal(&claim, Lapse::Revoked(claim.epoch)));
        };
        self.check_claim(&record.claim(credential), at)
    }

    /// The refusal for a lapsed claim, naming the principal's current epoch on revocation.
    fn refusal(&self, claim: &Claim, lapse: Lapse) -> AuthzError {
        match lapse {
            Lapse::Revoked(epoch) => AuthzError::CredentialRevoked {
                credential: claim.credential,
                epoch,
                principal_epoch: self.principals.current(&claim.principal),
            },
            Lapse::Expired(expired_at) => AuthzError::CredentialExpired {
                credential: claim.credential,
                expired_at,
            },
        }
    }
}
