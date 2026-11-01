//! Rotating and withdrawing credentials under an administrator's grant, and rechecking the
//! credential behind a session for each request (plan todo 57).

use ovis_terminal_protocol::schema::scalar::TimestampMs;

use super::issuance::within_issuer;
use super::table::NewCredential;
use super::{AccessRequest, Authority, Session};
use crate::credential::{IssueRequest, Revocation};
use crate::error::AuthzError;
use crate::family::IdentityFamily;
use crate::permission::Action;
use crate::rotation::{PrincipalEpoch, Rotated, Rotation, Withdrawn};

impl<F: IdentityFamily> Authority<F> {
    /// Rechecks the credential behind `session` at `at`: the connection itself holds no
    /// authority, so a revoked or expired credential refuses every further request.
    ///
    /// # Errors
    ///
    /// [`AuthzError::CredentialRevoked`] naming the principal's current epoch,
    /// [`AuthzError::CredentialExpired`], or [`AuthzError::InvalidCredential`] for a session
    /// of another authority.
    pub fn validate(
        &self,
        session: &Session,
        at: TimestampMs,
    ) -> Result<PrincipalEpoch, AuthzError> {
        self.own(session)?;
        let table = self.read()?;
        table.check_claim(&session.claim, at)?;
        Ok(table.principals.current(session.principal()))
    }

    /// Withdraws a credential and advances its principal's epoch; sessions, decisions and
    /// queued writes made under it fail their next check.
    ///
    /// # Errors
    ///
    /// [`AuthzError::Forbidden`] when `admin` may not revoke and
    /// [`AuthzError::CredentialNotFound`] for an unknown credential.
    pub fn withdraw(
        &self,
        admin: &Session,
        revocation: &Revocation,
    ) -> Result<Withdrawn, AuthzError> {
        self.own(admin)?;
        let request = AccessRequest::new(
            Action::RevokeCredential,
            self.machine_target(),
            revocation.at,
        );
        let mut table = self.write()?;
        table.permit(admin, &request)?;
        let (epoch, principal, principal_epoch) = table.revoke(revocation.credential)?;
        drop(table);
        Ok(Withdrawn {
            credential: revocation.credential,
            epoch,
            principal,
            principal_epoch,
        })
    }

    /// Issues a replacement with the same principal and grants and withdraws the old
    /// credential under one lock, so no instant exists where both or neither authenticate.
    ///
    /// # Errors
    ///
    /// [`AuthzError::Forbidden`] when `admin` may not issue, the credential errors when the
    /// replaced credential is not live, [`AuthzError::PrincipalEpochConflict`] when the
    /// principal moved past the expected epoch, [`AuthzError::GrantExceedsIssuer`] and
    /// [`AuthzError::InvalidValue`] for an expiry beyond the issuer's or not in the future,
    /// and [`AuthzError::LimitExceeded`] when the table is full.
    pub fn rotate(&self, admin: &Session, rotation: &Rotation) -> Result<Rotated<F>, AuthzError> {
        let at = rotation.at;
        self.own(admin)?;
        let request = AccessRequest::new(Action::IssueCredential, self.machine_target(), at);
        let mut table = self.write()?;
        table.permit(admin, &request)?;
        let replaced = table.live(rotation.credential, at)?;
        let current = table.principals.current(&replaced.principal);
        if current != rotation.expected {
            return Err(AuthzError::PrincipalEpochConflict {
                principal_epoch: current,
            });
        }
        let expires_at = rotation.expires_at.unwrap_or(replaced.expires_at);
        if expires_at.get() <= at.get() {
            return Err(AuthzError::InvalidValue {
                field_path: "expires_at_ms",
            });
        }
        let replacement = IssueRequest::new(
            replaced.principal.clone(),
            replaced.grants.clone(),
            expires_at,
        );
        within_issuer(
            table.check_claim(&admin.claim, at)?,
            &replacement,
            &self.binding.machine,
        )?;
        table.make_room(at);
        let issued = table.insert(
            &self.binding.lifetime,
            NewCredential {
                principal: replacement.principal,
                grants: replacement.grants,
                expires_at,
            },
        )?;
        let (epoch, principal, principal_epoch) = table.revoke(rotation.credential)?;
        drop(table);
        Ok(Rotated {
            issued,
            withdrawn: Withdrawn {
                credential: rotation.credential,
                epoch,
                principal,
                principal_epoch,
            },
        })
    }
}
