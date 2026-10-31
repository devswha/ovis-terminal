//! Authentication, authorization and the commit-time recheck of decisions.

mod cell;
mod issuance;
mod issued;
mod secret;
mod table;
mod tombstone;

use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

pub use issued::IssuedCredential;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use secret::PresentedBearer;
use table::{Claim, CredentialTable};

use crate::config::{AuthorityLimits, RuntimeBinding};
use crate::endpoint::{PeerAdmission, Transport};
use crate::error::{AuthzError, MismatchReason};
use crate::family::IdentityFamily;
use crate::identity::PrincipalId;
use crate::permission::Action;
use crate::scope::ResolvedTarget;

/// Identity of one authority instance; a decision from another instance never commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AuthorityId(u128);

impl AuthorityId {
    fn random() -> Self {
        Self(uuid::Uuid::new_v4().as_u128())
    }
}

/// Issues, authenticates and checks credentials of one runtime lifetime.
///
/// Nothing is allowed by default: endpoint admission identifies a peer but grants no
/// permission, every request needs a grant covering its action and resolved target, and
/// every commit rechecks the decision it was admitted under.
#[derive(Debug)]
pub struct Authority<F: IdentityFamily> {
    id: AuthorityId,
    binding: RuntimeBinding<F>,
    limits: AuthorityLimits,
    table: RwLock<CredentialTable<F>>,
}

/// An authenticated connection: one principal under one credential epoch. It is not
/// `Clone`, so another client cannot inherit it.
#[derive(Debug)]
pub struct Session {
    authority: AuthorityId,
    claim: Claim,
    principal: PrincipalId,
    transport: Transport,
}

impl Session {
    /// The authenticated principal.
    #[must_use]
    pub const fn principal(&self) -> &PrincipalId {
        &self.principal
    }

    /// The transport the principal connected over.
    #[must_use]
    pub const fn transport(&self) -> Transport {
        self.transport
    }
}

/// Permission to perform one action on one resolved target, bound to the credential epoch
/// it was granted under. It cannot be built outside this crate or copied.
#[derive(Debug)]
pub struct Decision<F: IdentityFamily> {
    authority: AuthorityId,
    claim: Claim,
    principal: PrincipalId,
    action: Action,
    target: ResolvedTarget<F>,
}

impl<F: IdentityFamily> Decision<F> {
    /// The principal the decision was made for.
    #[must_use]
    pub const fn principal(&self) -> &PrincipalId {
        &self.principal
    }

    /// The target the decision covers.
    #[must_use]
    pub const fn target(&self) -> &ResolvedTarget<F> {
        &self.target
    }
}

/// Bearer text presented at one instant.
#[derive(Debug, Clone, Copy)]
pub struct Presentation<'a> {
    bearer: &'a str,
    at: TimestampMs,
}

impl<'a> Presentation<'a> {
    /// Presents `bearer` at `at`.
    #[must_use]
    pub const fn new(bearer: &'a str, at: TimestampMs) -> Self {
        Self { bearer, at }
    }
}

/// One action on one resolved target at one instant: what a request asks for, or what a
/// commit is about to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessRequest<F: IdentityFamily> {
    action: Action,
    target: ResolvedTarget<F>,
    at: TimestampMs,
}

impl<F: IdentityFamily> AccessRequest<F> {
    /// Asks to perform `action` on `target` at `at`.
    #[must_use]
    pub const fn new(action: Action, target: ResolvedTarget<F>, at: TimestampMs) -> Self {
        Self { action, target, at }
    }
}

/// The action and target a commit or queued write is about to perform, at the instant of
/// the write.
pub type CommitCheck<F> = AccessRequest<F>;

impl<F: IdentityFamily> Authority<F> {
    /// Authenticates bearer text over an admitted endpoint connection.
    ///
    /// # Errors
    ///
    /// [`AuthzError::Unauthenticated`] for an empty bearer, [`AuthzError::InvalidCredential`]
    /// for text this authority did not issue, [`AuthzError::CredentialRevoked`] or
    /// [`AuthzError::CredentialExpired`] for a credential no longer valid, and
    /// [`AuthzError::Poisoned`] when the table is unusable.
    pub fn authenticate(
        &self,
        admission: &PeerAdmission,
        presentation: Presentation<'_>,
    ) -> Result<Session, AuthzError> {
        let bearer = PresentedBearer::parse(presentation.bearer)?;
        let (claim, principal) = {
            let table = self.read()?;
            let record = table
                .records
                .get(&bearer.credential)
                .filter(|record| record.proves(&bearer.secret))
                .ok_or(AuthzError::InvalidCredential)?;
            let claim = record.claim(bearer.credential);
            record.check(&claim, presentation.at)?;
            let principal = record.principal.clone();
            drop(table);
            (claim, principal)
        };
        Ok(Session {
            authority: self.id,
            claim,
            principal,
            transport: admission.transport(),
        })
    }

    /// Decides whether `session` may perform the requested action on its target.
    ///
    /// # Errors
    ///
    /// [`AuthzError::Forbidden`] when no grant covers the request, the credential errors of
    /// [`Authority::authenticate`] when the credential lapsed since authentication.
    pub fn authorize(
        &self,
        session: &Session,
        request: &AccessRequest<F>,
    ) -> Result<Decision<F>, AuthzError> {
        self.own(session)?;
        self.read()?.permit(session, request)
    }

    /// Rechecks `decision` at the commit or write boundary of `commit`.
    ///
    /// # Errors
    ///
    /// [`AuthzError::DecisionMismatch`] when the commit is not the decided action and target
    /// of this authority, [`AuthzError::CredentialRevoked`] or
    /// [`AuthzError::CredentialExpired`] when the credential lapsed after the decision.
    pub fn recheck(
        &self,
        decision: &Decision<F>,
        commit: &CommitCheck<F>,
    ) -> Result<(), AuthzError> {
        let mismatch = if decision.authority != self.id {
            Some(MismatchReason::Authority)
        } else if decision.action != commit.action {
            Some(MismatchReason::Action)
        } else if decision.target != commit.target {
            Some(MismatchReason::Target)
        } else {
            None
        };
        if let Some(reason) = mismatch {
            return Err(AuthzError::DecisionMismatch { reason });
        }
        self.read()?
            .check_claim(&decision.claim, commit.at)
            .map(|_| ())
    }

    fn own(&self, session: &Session) -> Result<(), AuthzError> {
        if session.authority == self.id {
            Ok(())
        } else {
            Err(AuthzError::InvalidCredential)
        }
    }

    fn read(&self) -> Result<RwLockReadGuard<'_, CredentialTable<F>>, AuthzError> {
        self.table.read().map_err(|_| AuthzError::Poisoned)
    }

    fn write(&self) -> Result<RwLockWriteGuard<'_, CredentialTable<F>>, AuthzError> {
        self.table.write().map_err(|_| AuthzError::Poisoned)
    }
}
