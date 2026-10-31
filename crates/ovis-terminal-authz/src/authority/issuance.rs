//! Bootstrapping the owner credential, issuing delegated credentials and revoking them.

use std::sync::RwLock;

use super::table::{CredentialRecord, CredentialTable, NewCredential};
use super::{AccessRequest, Authority, AuthorityId, IssuedCredential, Session};
use crate::config::AuthorityConfig;
use crate::credential::{Issuance, IssueRequest, Revocation};
use crate::error::{AuthzError, Exceeded, Limit};
use crate::family::IdentityFamily;
use crate::identity::CredentialEpoch;
use crate::permission::{Action, Role};
use crate::scope::{Grant, ResolvedTarget, ResourceScope, ScopeResource, TargetResource};

impl<F: IdentityFamily> Authority<F> {
    /// Creates an authority and its owner credential, which holds every permission over
    /// the whole machine until the configured expiry.
    ///
    /// # Errors
    ///
    /// [`AuthzError::LimitExceeded`] when the limits cannot hold the owner credential.
    pub fn bootstrap(
        config: AuthorityConfig<F>,
    ) -> Result<(Self, IssuedCredential<F>), AuthzError> {
        let machine = config.binding.machine.clone();
        let grants: Vec<Grant<F>> = Role::Owner
            .permissions()
            .iter()
            .map(|permission| {
                let scope = ResourceScope::new(machine.clone(), ScopeResource::Machine);
                Grant::new(*permission, scope)
            })
            .collect();
        if grants.len() > config.limits.grants {
            return Err(AuthzError::LimitExceeded {
                limit: Limit::Grants,
            });
        }
        let mut table = CredentialTable::new(config.limits);
        let owner = NewCredential {
            principal: config.owner.principal,
            grants,
            expires_at: config.owner.expires_at,
        };
        let issued = table.insert(&config.binding.lifetime, owner)?;
        let authority = Self {
            id: AuthorityId::random(),
            binding: config.binding,
            limits: config.limits,
            table: RwLock::new(table),
        };
        Ok((authority, issued))
    }

    /// Issues a credential to another principal; `issuer` needs the admin permission and
    /// may only delegate grants, machine scope and expiry it holds itself.
    ///
    /// # Errors
    ///
    /// [`AuthzError::Forbidden`] when `issuer` may not issue, [`AuthzError::GrantExceedsIssuer`]
    /// for a grant or expiry beyond the issuer's, [`AuthzError::InvalidValue`] for an expiry
    /// not after the issuance instant, and [`AuthzError::LimitExceeded`] when a table is full.
    pub fn issue(
        &self,
        issuer: &Session,
        issuance: Issuance<F>,
    ) -> Result<IssuedCredential<F>, AuthzError> {
        let Issuance { request, at } = issuance;
        self.own(issuer)?;
        let admin = AccessRequest::new(Action::IssueCredential, self.machine_target(), at);
        let mut table = self.write()?;
        table.permit(issuer, &admin)?;
        within_issuer(
            table.check_claim(issuer.claim, at)?,
            &request,
            &self.binding.machine,
        )?;
        if request.grants.len() > self.limits.grants {
            return Err(AuthzError::LimitExceeded {
                limit: Limit::Grants,
            });
        }
        if request.expires_at_ms.get() <= at.get() {
            return Err(AuthzError::InvalidValue {
                field_path: "expires_at_ms",
            });
        }
        let new = NewCredential {
            principal: request.principal,
            grants: request.grants,
            expires_at: request.expires_at_ms,
        };
        table.make_room(at);
        table.insert(&self.binding.lifetime, new)
    }

    /// Revokes a credential: its sessions, decisions and queued writes fail their next check.
    ///
    /// # Errors
    ///
    /// [`AuthzError::Forbidden`] when `admin` may not revoke and
    /// [`AuthzError::CredentialNotFound`] for an unknown credential.
    pub fn revoke(
        &self,
        admin: &Session,
        revocation: &Revocation,
    ) -> Result<CredentialEpoch, AuthzError> {
        self.own(admin)?;
        let request = AccessRequest::new(
            Action::RevokeCredential,
            self.machine_target(),
            revocation.at,
        );
        let mut table = self.write()?;
        table.permit(admin, &request)?;
        let epoch = table.revoke(revocation.credential)?;
        drop(table);
        Ok(epoch)
    }

    fn machine_target(&self) -> ResolvedTarget<F> {
        ResolvedTarget::new(self.binding.machine.clone(), TargetResource::Machine)
    }
}

fn within_issuer<F: IdentityFamily>(
    issuer: &CredentialRecord<F>,
    request: &IssueRequest<F>,
    machine: &F::Machine,
) -> Result<(), AuthzError> {
    for grant in &request.grants {
        let exceeded = if grant.machine() != machine {
            Some(Exceeded::Machine)
        } else if issuer.grants.iter().any(|held| held.includes(grant)) {
            None
        } else {
            Some(Exceeded::Permission(grant.permission()))
        };
        if let Some(exceeded) = exceeded {
            return Err(AuthzError::GrantExceedsIssuer { exceeded });
        }
    }
    if request.expires_at_ms.get() > issuer.expires_at.get() {
        return Err(AuthzError::GrantExceedsIssuer {
            exceeded: Exceeded::Expiry,
        });
    }
    Ok(())
}
