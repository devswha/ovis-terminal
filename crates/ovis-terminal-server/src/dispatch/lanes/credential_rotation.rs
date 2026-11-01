//! `credential.rotate` and `credential.revoke` (plan todo 57).
//!
//! Both need the admin permission on the machine, decided at admission and rechecked by
//! the authority under its own lock when the change is made. The change, the recheck of
//! every queued write and the operation receipt form one commit unit; a refusal is kept as
//! the operation's typed failure, so a retry with the same operation id answers it again.

use std::fmt::Debug;
use std::sync::Arc;

use ovis_terminal_authz::{
    Action, Authority, AuthzError, CredentialId, IssuedCredential, ResolvedTarget, Revocation,
    Rotation, Session, TargetResource, Withdrawn,
};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{Completion, OperationEvent, OperationSnapshot};
use ovis_terminal_core::registry::{MachineId, RegistryIdentities};
use ovis_terminal_protocol::domain::credential_rotation::{
    BearerText, CredentialRevokeParams, CredentialRevokeResult, CredentialRotateParams,
    CredentialRotateResult, IssuedReplacement, PrincipalStanding, RacedWrite, WithdrawnCredential,
};
use ovis_terminal_protocol::schema::ids::{self, PrincipalName};
use ovis_terminal_protocol::schema::scalar::{self, TimestampMs};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::dispatch::execution::now;
use crate::dispatch::{Access, Completed, DispatchError, Execution, Method};

/// Writes admitted but not yet written by the runtime's terminals.
pub trait QueuedWrites: Debug + Send + Sync + 'static {
    /// Rechecks every queued write against the current authority, fencing those that no
    /// longer hold with zero bytes, and reports each write the recheck settled or left with
    /// the holder.
    fn recheck(&self) -> Vec<RacedWrite>;
}

/// Shared state of the credential methods.
#[derive(Debug)]
pub struct CredentialLane {
    authority: Arc<Authority<RegistryIdentities>>,
    machine: MachineId,
    writes: Arc<dyn QueuedWrites>,
}

impl CredentialLane {
    /// Changes credentials of `authority` on `machine`, rechecking `writes` after each.
    pub fn new(
        authority: Arc<Authority<RegistryIdentities>>,
        machine: MachineId,
        writes: Arc<dyn QueuedWrites>,
    ) -> Arc<Self> {
        Arc::new(Self {
            authority,
            machine,
            writes,
        })
    }

    const fn access(&self, action: Action) -> Access {
        Access::new(
            action,
            ResolvedTarget::new(self.machine, TargetResource::Machine),
        )
    }

    fn revoked(&self, withdrawn: &Withdrawn) -> CredentialRevokeResult {
        CredentialRevokeResult::new(
            WithdrawnCredential::new(
                wire_credential(withdrawn.credential()),
                scalar::CredentialEpoch::new(withdrawn.epoch().get()),
            ),
            PrincipalStanding::new(
                PrincipalName::new(withdrawn.principal().as_str().to_owned()),
                withdrawn.principal_epoch(),
            ),
            self.writes.recheck(),
        )
    }

    fn rotate(
        &self,
        admin: &Session,
        (params, at): (&CredentialRotateParams, TimestampMs),
    ) -> Result<CredentialRotateResult, AuthzError> {
        let mut rotation =
            Rotation::new(credential(&params.credential)?, params.principal_epoch, at);
        if let Some(expires_at) = params.expires_at_ms {
            rotation = rotation.until(expires_at);
        }
        let rotated = self.authority.rotate(admin, &rotation)?;
        Ok(CredentialRotateResult::new(
            replacement(rotated.issued()),
            self.revoked(rotated.withdrawn()),
        ))
    }

    fn revoke(
        &self,
        admin: &Session,
        (params, at): (&CredentialRevokeParams, TimestampMs),
    ) -> Result<CredentialRevokeResult, AuthzError> {
        let revocation = Revocation::new(credential(&params.credential)?, at);
        let withdrawn = self.authority.withdraw(admin, &revocation)?;
        Ok(self.revoked(&withdrawn))
    }
}

fn credential(text: &ids::CredentialId) -> Result<CredentialId, AuthzError> {
    CredentialId::parse(text.as_str()).map_err(|_| AuthzError::InvalidValue {
        field_path: "params.credential",
    })
}

fn wire_credential(credential: CredentialId) -> ids::CredentialId {
    ids::CredentialId::new(credential.to_string())
}

fn replacement(issued: &IssuedCredential<RegistryIdentities>) -> IssuedReplacement {
    IssuedReplacement::new(
        (
            wire_credential(*issued.id()),
            scalar::CredentialEpoch::new(issued.epoch().get()),
        ),
        issued.expires_at(),
        BearerText::new(issued.bearer().to_owned()),
    )
}

/// The completion of a credential change: its result, or its refusal kept as the
/// operation's typed failure.
fn completion<D, R: Serialize>(
    outcome: Result<R, AuthzError>,
) -> Result<Completion<D>, ovis_terminal_core::ops::OperationError> {
    match outcome {
        Ok(result) => Ok(Completion::new(serde_json::to_value(result)?)),
        Err(error) => Ok(Completion::failed(
            DispatchError::Authority(error).boundary().data().clone(),
        )),
    }
}

/// `credential.rotate`: replace a credential and withdraw the old one in one step.
#[derive(Debug)]
pub struct RotateCredential(Arc<CredentialLane>);

/// `credential.revoke`: withdraw a credential and recheck its queued writes.
#[derive(Debug)]
pub struct RevokeCredential(Arc<CredentialLane>);

impl RotateCredential {
    /// The method over `lane`'s shared state.
    #[must_use]
    pub fn new(lane: &Arc<CredentialLane>) -> Self {
        Self(Arc::clone(lane))
    }
}

impl RevokeCredential {
    /// The method over `lane`'s shared state.
    #[must_use]
    pub fn new(lane: &Arc<CredentialLane>) -> Self {
        Self(Arc::clone(lane))
    }
}

macro_rules! credential_method {
    ($name:ident, $method:literal, $params:ty, $action:expr, $change:ident) => {
        impl<D, P> Method<D, P> for $name
        where
            D: EventData
                + From<OperationEvent>
                + Clone
                + Serialize
                + DeserializeOwned
                + Send
                + 'static,
            P: SnapshotProducer + Send + Sync + 'static,
            P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
        {
            const NAME: &'static str = $method;
            type Params = $params;

            fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
                Ok(self.0.access($action))
            }

            async fn run(
                &self,
                params: Self::Params,
                execution: Execution<D, P>,
            ) -> Result<Completed, DispatchError> {
                let lane = Arc::clone(&self.0);
                let admin = Arc::clone(execution.session());
                let at = now()?;
                execution
                    .commit_unit(move |_| completion(lane.$change(&admin, (&params, at))))
                    .await
            }
        }
    };
}

credential_method!(
    RotateCredential,
    "credential.rotate",
    CredentialRotateParams,
    Action::IssueCredential,
    rotate
);
credential_method!(
    RevokeCredential,
    "credential.revoke",
    CredentialRevokeParams,
    Action::RevokeCredential,
    revoke
);
