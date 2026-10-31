//! The in-memory credential table of one authority.

use std::collections::HashMap;
use std::sync::Arc;

use ovis_terminal_protocol::schema::scalar::TimestampMs;

use super::cell::RevocationCell;
use super::issued::{Bearer, IssuedCredential};
use super::secret::CredentialSecret;
use super::tombstone::{Tombstone, Tombstones};
use super::{AccessRequest, Decision, Session};
use crate::config::AuthorityLimits;
use crate::error::{AuthzError, Limit};
use crate::family::IdentityFamily;
use crate::identity::{CredentialEpoch, CredentialId, PrincipalId};
use crate::permission::Action;
use crate::scope::{Grant, TargetResource};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Standing {
    Active,
    Revoked,
}

#[derive(Debug)]
pub(super) struct CredentialRecord<F: IdentityFamily> {
    pub(super) principal: PrincipalId,
    secret: CredentialSecret,
    pub(super) grants: Vec<Grant<F>>,
    pub(super) expires_at: TimestampMs,
    pub(super) epoch: CredentialEpoch,
    standing: Standing,
    revocation: Arc<RevocationCell>,
}

/// The credential a session or decision was made under, with the cause-bearing facts bound
/// at that moment: its epoch, its expiry and the revocation cell shared with the record.
#[derive(Debug, Clone)]
pub(super) struct Claim {
    pub(super) credential: CredentialId,
    pub(super) epoch: CredentialEpoch,
    expires_at: TimestampMs,
    revocation: Arc<RevocationCell>,
}

impl Claim {
    /// Fails with the bound cause, without consulting the table, so reclaiming the record or
    /// its tombstone cannot turn a revoked or expired claim into an unknown one.
    fn check_bound(&self, at: TimestampMs) -> Result<(), AuthzError> {
        if let Some(epoch) = self.revocation.revoked_at() {
            return Err(AuthzError::CredentialRevoked {
                credential: self.credential,
                epoch,
            });
        }
        if at.get() >= self.expires_at.get() {
            return Err(AuthzError::CredentialExpired {
                credential: self.credential,
                expired_at: self.expires_at,
            });
        }
        Ok(())
    }
}

impl<F: IdentityFamily> CredentialRecord<F> {
    pub(super) fn claim(&self, credential: CredentialId) -> Claim {
        Claim {
            credential,
            epoch: self.epoch,
            expires_at: self.expires_at,
            revocation: Arc::clone(&self.revocation),
        }
    }

    /// Fails unless the credential is still active at the claimed epoch and unexpired at `at`.
    pub(super) const fn check(&self, claim: &Claim, at: TimestampMs) -> Result<(), AuthzError> {
        if matches!(self.standing, Standing::Revoked) || self.epoch.get() != claim.epoch.get() {
            return Err(AuthzError::CredentialRevoked {
                credential: claim.credential,
                epoch: self.epoch,
            });
        }
        if at.get() >= self.expires_at.get() {
            return Err(AuthzError::CredentialExpired {
                credential: claim.credential,
                expired_at: self.expires_at,
            });
        }
        Ok(())
    }

    /// The bearer text this record was issued as, the key of its attestations.
    pub(super) fn bearer_text(&self, credential: CredentialId) -> String {
        self.secret.bearer(credential)
    }

    pub(super) fn proves(&self, presented: &CredentialSecret) -> bool {
        self.secret.matches(presented)
    }

    pub(super) fn revoke(&mut self) -> CredentialEpoch {
        self.epoch = self.epoch.next();
        self.standing = Standing::Revoked;
        self.revocation.revoke(self.epoch);
        self.epoch
    }

    const fn live_at(&self, at: TimestampMs) -> bool {
        matches!(self.standing, Standing::Active) && at.get() < self.expires_at.get()
    }

    const fn tombstone(&self, credential: CredentialId) -> Tombstone {
        Tombstone {
            credential,
            epoch: self.epoch,
            expires_at: self.expires_at,
        }
    }
}

pub(super) struct NewCredential<F: IdentityFamily> {
    pub(super) principal: PrincipalId,
    pub(super) grants: Vec<Grant<F>>,
    pub(super) expires_at: TimestampMs,
}

#[derive(Debug)]
pub(super) struct CredentialTable<F: IdentityFamily> {
    last_issued: u64,
    capacity: usize,
    pub(super) records: HashMap<CredentialId, CredentialRecord<F>>,
    tombstones: Tombstones,
}

impl<F: IdentityFamily> CredentialTable<F> {
    pub(super) fn new(limits: AuthorityLimits) -> Self {
        Self {
            last_issued: 0,
            capacity: limits.credentials,
            records: HashMap::new(),
            tombstones: Tombstones::new(limits.tombstones),
        }
    }

    /// Returns the record behind `claim` if it is still valid at `at`; a claim on a
    /// reclaimed revoked credential still fails as revoked.
    pub(super) fn check_claim(
        &self,
        claim: &Claim,
        at: TimestampMs,
    ) -> Result<&CredentialRecord<F>, AuthzError> {
        claim.check_bound(at)?;
        let Some(record) = self.records.get(&claim.credential) else {
            return Err(self.tombstones.get(claim.credential).map_or(
                AuthzError::InvalidCredential,
                |tombstone| AuthzError::CredentialRevoked {
                    credential: claim.credential,
                    epoch: tombstone.epoch,
                },
            ));
        };
        record.check(claim, at)?;
        Ok(record)
    }

    /// Revokes `credential`; revoking an already reclaimed revocation reports its epoch.
    pub(super) fn revoke(
        &mut self,
        credential: CredentialId,
    ) -> Result<CredentialEpoch, AuthzError> {
        if let Some(record) = self.records.get_mut(&credential) {
            return Ok(record.revoke());
        }
        self.tombstones
            .get(credential)
            .map(|tombstone| tombstone.epoch)
            .ok_or(AuthzError::CredentialNotFound { credential })
    }

    /// Decides `request` for a session this table's authority issued.
    pub(super) fn permit(
        &self,
        session: &Session,
        request: &AccessRequest<F>,
    ) -> Result<Decision<F>, AuthzError> {
        let record = self.check_claim(&session.claim, request.at)?;
        let required = request.action.required();
        let allowed = match (request.target.resource(), request.action) {
            (
                TargetResource::Operation { principal, .. },
                Action::InspectOperation | Action::CancelOperation,
            ) => {
                principal == &session.principal
                    && record.grants.iter().any(|grant| {
                        grant.machine() == request.target.machine()
                            && grant.permission() == required
                    })
            }
            (TargetResource::Operation { .. }, _)
            | (_, Action::InspectOperation | Action::CancelOperation) => false,
            _ => record
                .grants
                .iter()
                .any(|grant| grant.allows(required, &request.target)),
        };
        if !allowed {
            return Err(AuthzError::Forbidden {
                principal: session.principal.clone(),
                action: request.action,
                required,
            });
        }
        Ok(Decision {
            authority: session.authority,
            claim: session.claim.clone(),
            principal: session.principal.clone(),
            action: request.action,
            target: request.target.clone(),
        })
    }

    /// When the table is full, drops records that can never pass a check again. A revoked,
    /// unexpired record is dropped only after its revocation is buried as a tombstone; if
    /// the tombstone set is full of unexpired revocations the record stays.
    pub(super) fn make_room(&mut self, at: TimestampMs) {
        if self.records.len() < self.capacity {
            return;
        }
        let tombstones = &mut self.tombstones;
        self.records.retain(|credential, record| {
            if record.live_at(at) {
                return true;
            }
            let revoked_unexpired =
                matches!(record.standing, Standing::Revoked) && at.get() < record.expires_at.get();
            revoked_unexpired && !tombstones.bury(record.tombstone(*credential), at)
        });
    }

    pub(super) fn insert(
        &mut self,
        lifetime: &F::Lifetime,
        new: NewCredential<F>,
    ) -> Result<IssuedCredential<F>, AuthzError> {
        let sequence = self.last_issued.checked_add(1);
        let (Some(sequence), true) = (sequence, self.records.len() < self.capacity) else {
            return Err(AuthzError::LimitExceeded {
                limit: Limit::Credentials,
            });
        };
        let credential = CredentialId::new(sequence);
        let secret = CredentialSecret::generate();
        let bearer = secret.bearer(credential);
        let issued = IssuedCredential {
            credential,
            principal: new.principal.clone(),
            runtime_lifetime: lifetime.clone(),
            epoch: CredentialEpoch::FIRST,
            expires_at_ms: new.expires_at,
            grants: new.grants.clone(),
            bearer: Bearer(bearer),
        };
        self.records.insert(
            credential,
            CredentialRecord {
                principal: new.principal,
                secret,
                grants: new.grants,
                expires_at: new.expires_at,
                epoch: CredentialEpoch::FIRST,
                standing: Standing::Active,
                revocation: Arc::default(),
            },
        );
        self.last_issued = sequence;
        Ok(issued)
    }
}
