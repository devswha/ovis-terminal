//! Named-pipe security descriptors and the first-instance lifecycle of a pipe name.
//!
//! The first instance of a name must be created with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so
//! creation fails if another process already squats the name. Additional instances serve
//! concurrent and reconnecting clients only while the runtime still holds one open
//! instance; once every instance closes the name is free again and must be reclaimed.

use super::WindowsSid;
use crate::error::{AuthzError, EndpointDefect};

/// How a pipe instance was created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PipeInstance {
    /// Created with `FILE_FLAG_FIRST_PIPE_INSTANCE`.
    First,
    /// Created while an earlier instance of the name was still open.
    Additional,
}

/// Whether the instance was created with `PIPE_REJECT_REMOTE_CLIENTS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RemoteClients {
    /// Remote clients are refused by the operating system.
    Rejected,
    /// Remote clients may connect.
    Accepted,
}

/// Whether an access-control entry allows or denies its trustee.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AceKind {
    /// Allows access.
    Allow,
    /// Denies access.
    Deny,
}

/// One access-control entry of a pipe DACL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ace {
    kind: AceKind,
    trustee: WindowsSid,
}

impl Ace {
    /// An entry of `kind` for `trustee`.
    #[must_use]
    pub const fn new(kind: AceKind, trustee: WindowsSid) -> Self {
        Self { kind, trustee }
    }
}

/// The discretionary access-control list of a pipe.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Dacl {
    /// A null DACL, which allows everyone.
    Null,
    /// An explicit list of entries.
    Entries(Vec<Ace>),
}

/// The owner and DACL of a pipe instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeSecurity {
    owner: WindowsSid,
    dacl: Dacl,
}

impl PipeSecurity {
    /// A descriptor owned by `owner` with `dacl`.
    #[must_use]
    pub const fn new(owner: WindowsSid, dacl: Dacl) -> Self {
        Self { owner, dacl }
    }
}

/// What the operating system reports about one pipe instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeInstanceEvidence {
    instance: PipeInstance,
    remote: RemoteClients,
    security: PipeSecurity,
}

impl PipeInstanceEvidence {
    /// An instance created as `instance`, with `remote` and `security`.
    #[must_use]
    pub const fn new(
        instance: PipeInstance,
        remote: RemoteClients,
        security: PipeSecurity,
    ) -> Self {
        Self {
            instance,
            remote,
            security,
        }
    }
}

pub(super) fn check_security(
    evidence: &PipeInstanceEvidence,
    owner: &WindowsSid,
) -> Result<(), AuthzError> {
    let defect = if evidence.security.owner != *owner {
        Some(EndpointDefect::ForeignOwner)
    } else if evidence.remote == RemoteClients::Accepted {
        Some(EndpointDefect::RemoteClients)
    } else {
        match &evidence.security.dacl {
            Dacl::Null => Some(EndpointDefect::NullDacl),
            Dacl::Entries(entries) => entries
                .iter()
                .any(|ace| ace.kind == AceKind::Allow && ace.trustee != *owner)
                .then_some(EndpointDefect::ForeignTrustee),
        }
    };
    defect.map_or(Ok(()), |defect| {
        Err(AuthzError::EndpointInsecure { defect })
    })
}

/// Tracks the open instances of one pipe name the runtime serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeNamespace {
    owner: WindowsSid,
    open: u64,
}

impl PipeNamespace {
    /// An unclaimed pipe name served for `owner`.
    #[must_use]
    pub const fn new(owner: WindowsSid) -> Self {
        Self { owner, open: 0 }
    }

    /// Records a newly created instance after checking its security and that it was
    /// created as the first instance exactly when no other instance is open.
    ///
    /// # Errors
    ///
    /// [`AuthzError::EndpointInsecure`] for an exposed descriptor or a lifecycle violation.
    pub fn open(&mut self, evidence: &PipeInstanceEvidence) -> Result<(), AuthzError> {
        check_security(evidence, &self.owner)?;
        let defect = match (evidence.instance, self.open) {
            (PipeInstance::Additional, 0) => Some(EndpointDefect::UnclaimedName),
            (PipeInstance::First, 1..) => Some(EndpointDefect::NameAlreadyClaimed),
            (PipeInstance::First, 0) | (PipeInstance::Additional, 1..) => None,
        };
        if let Some(defect) = defect {
            return Err(AuthzError::EndpointInsecure { defect });
        }
        // Instance counts are bounded by the operating system (at most 255 per name).
        self.open = self.open.saturating_add(1);
        Ok(())
    }

    /// Records that one instance closed, after its client disconnected.
    ///
    /// # Errors
    ///
    /// [`AuthzError::EndpointInsecure`] when no instance is open.
    pub fn close(&mut self) -> Result<(), AuthzError> {
        self.open = self
            .open
            .checked_sub(1)
            .ok_or(AuthzError::EndpointInsecure {
                defect: EndpointDefect::InstanceUnderflow,
            })?;
        Ok(())
    }
}
