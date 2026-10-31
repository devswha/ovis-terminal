//! The in-memory credential table of one authority.

use std::collections::HashMap;

use ovis_terminal_protocol::schema::ids::RuntimeLifetimeId;
use ovis_terminal_protocol::schema::scalar::TimestampMs;

use super::issued::{Bearer, IssuedCredential};
use super::secret::CredentialSecret;
use super::{AccessRequest, Decision, Session};
use crate::error::{AuthzError, Limit};
use crate::identity::{CredentialEpoch, CredentialId, PrincipalId};
use crate::scope::Grant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Standing {
    Active,
    Revoked,
}

#[derive(Debug)]
pub(super) struct CredentialRecord {
    pub(super) principal: PrincipalId,
    secret: CredentialSecret,
    pub(super) grants: Vec<Grant>,
    pub(super) expires_at: TimestampMs,
    pub(super) epoch: CredentialEpoch,
    standing: Standing,
}

/// The credential and epoch a session or decision was made under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Claim {
    pub(super) credential: CredentialId,
    pub(super) epoch: CredentialEpoch,
}

impl CredentialRecord {
    /// Fails unless the credential is still active at the claimed epoch and unexpired at `at`.
    pub(super) const fn check(&self, claim: Claim, at: TimestampMs) -> Result<(), AuthzError> {
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

    pub(super) fn proves(&self, presented: &CredentialSecret) -> bool {
        self.secret.matches(presented)
    }

    pub(super) const fn revoke(&mut self) -> CredentialEpoch {
        self.epoch = self.epoch.next();
        self.standing = Standing::Revoked;
        self.epoch
    }

    const fn live_at(&self, at: TimestampMs) -> bool {
        matches!(self.standing, Standing::Active) && at.get() < self.expires_at.get()
    }
}

pub(super) struct NewCredential {
    pub(super) principal: PrincipalId,
    pub(super) grants: Vec<Grant>,
    pub(super) expires_at: TimestampMs,
}

#[derive(Debug)]
pub(super) struct CredentialTable {
    last_issued: u64,
    capacity: usize,
    pub(super) records: HashMap<CredentialId, CredentialRecord>,
}

impl CredentialTable {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            last_issued: 0,
            capacity,
            records: HashMap::new(),
        }
    }

    /// Returns the record behind `claim` if it is still valid at `at`.
    pub(super) fn check_claim(
        &self,
        claim: Claim,
        at: TimestampMs,
    ) -> Result<&CredentialRecord, AuthzError> {
        let record = self
            .records
            .get(&claim.credential)
            .ok_or(AuthzError::InvalidCredential)?;
        record.check(claim, at)?;
        Ok(record)
    }

    /// Decides `request` for a session this table's authority issued.
    pub(super) fn permit(
        &self,
        session: &Session,
        request: &AccessRequest,
    ) -> Result<Decision, AuthzError> {
        let record = self.check_claim(session.claim, request.at)?;
        let required = request.action.required();
        if !record
            .grants
            .iter()
            .any(|grant| grant.allows(required, &request.target))
        {
            return Err(AuthzError::Forbidden {
                principal: session.principal.clone(),
                action: request.action,
                required,
            });
        }
        Ok(Decision {
            authority: session.authority,
            claim: session.claim,
            principal: session.principal.clone(),
            action: request.action,
            target: request.target.clone(),
        })
    }

    /// Drops revoked and expired records when the table is full; they can never pass a
    /// check again, so forgetting them only turns their errors into `invalid_credential`.
    pub(super) fn make_room(&mut self, at: TimestampMs) {
        if self.records.len() >= self.capacity {
            self.records.retain(|_, record| record.live_at(at));
        }
    }

    pub(super) fn insert(
        &mut self,
        lifetime: &RuntimeLifetimeId,
        new: NewCredential,
    ) -> Result<IssuedCredential, AuthzError> {
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
            },
        );
        self.last_issued = sequence;
        Ok(issued)
    }
}
