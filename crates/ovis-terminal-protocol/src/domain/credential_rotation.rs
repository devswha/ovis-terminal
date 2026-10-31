//! `credential.rotate` and `credential.revoke` and the fate of writes they race (plan todo 57).
//!
//! Rotation issues a replacement for the same principal, grants and expiry and withdraws
//! the old credential in one step, guarded by the principal epoch the caller last read.
//! Writes still queued under a withdrawn credential are rechecked before any byte reaches
//! the program; each one is reported as cancelled before commit, already committed with its
//! delivery facts, or of unknown outcome while the holder has not answered.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::errors::OpId;
use crate::schema::ids::{CredentialId, PrincipalName};
use crate::schema::method::{MethodSpec, method};
use crate::schema::scalar::{
    ByteCount, ByteOffset, CredentialEpoch, EventSeq, PrincipalEpoch, TimestampMs,
};

/// Parameters of `credential.rotate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CredentialRotateParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// The credential to replace.
    pub credential: CredentialId,
    /// The principal epoch the caller last read; a newer one refuses the rotation.
    pub principal_epoch: PrincipalEpoch,
    /// Expiry of the replacement; defaults to the replaced credential's expiry.
    #[serde(default)]
    pub expires_at_ms: Option<TimestampMs>,
}

/// Parameters of `credential.revoke`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CredentialRevokeParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// The credential to withdraw.
    pub credential: CredentialId,
}

/// Bearer text, shown to the caller exactly once and never in diagnostics.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct BearerText(String);

impl BearerText {
    /// Wraps issued bearer text.
    #[must_use]
    pub const fn new(text: String) -> Self {
        Self(text)
    }

    /// Borrows the bearer text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BearerText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BearerText(<redacted>)")
    }
}

/// A credential that no longer authenticates, and the epoch its withdrawal produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct WithdrawnCredential {
    /// The withdrawn credential.
    pub credential: CredentialId,
    /// Its epoch after withdrawal.
    pub epoch: CredentialEpoch,
}

impl WithdrawnCredential {
    /// `credential`, withdrawn at `epoch`.
    #[must_use]
    pub const fn new(credential: CredentialId, epoch: CredentialEpoch) -> Self {
        Self { credential, epoch }
    }
}

/// Stable token of the reason no byte of a write was delivered, such as
/// `credential_revoked`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct FenceToken(String);

impl FenceToken {
    /// Wraps a fence reason token.
    #[must_use]
    pub const fn new(token: String) -> Self {
        Self(token)
    }

    /// Borrows the token.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Where a write queued under a withdrawn credential ended up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum CommitRace {
    /// Rechecked before its write; no byte reached the program.
    CancelledBeforeCommit {
        /// Why the write was withheld.
        reason: FenceToken,
    },
    /// The holder had already written these bytes before the withdrawal.
    AlreadyCommitted {
        /// Bytes the holder wrote.
        written_bytes: ByteCount,
        /// Output offset the holder saw when the write started.
        output_offset: ByteOffset,
    },
    /// The write is with the holder and its fence is not answered yet; it is never resent.
    OutcomeUnknown,
}

/// One write raced by a withdrawal: its admission receipt and its fate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct RacedWrite {
    /// Operation that admitted the write.
    pub op_id: OpId,
    /// Its position in the terminal's input sequence.
    pub queued_seq: EventSeq,
    /// Bytes admitted.
    pub admitted_bytes: ByteCount,
    /// What happened to them.
    pub race: CommitRace,
}

impl RacedWrite {
    /// The write `op_id` admitted at `queued_seq` with `admitted_bytes`, and its fate.
    #[must_use]
    pub fn new(
        (op_id, queued_seq): (OpId, EventSeq),
        admitted_bytes: ByteCount,
        race: CommitRace,
    ) -> Self {
        Self {
            op_id,
            queued_seq,
            admitted_bytes,
            race,
        }
    }
}

/// The principal a withdrawal touched and its epoch afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct PrincipalStanding {
    /// The principal.
    pub principal: PrincipalName,
    /// Its epoch after the change.
    pub principal_epoch: PrincipalEpoch,
}

impl PrincipalStanding {
    /// `principal` at `principal_epoch`.
    #[must_use]
    pub const fn new(principal: PrincipalName, principal_epoch: PrincipalEpoch) -> Self {
        Self {
            principal,
            principal_epoch,
        }
    }
}

/// Result of `credential.revoke`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct CredentialRevokeResult {
    /// The withdrawn credential.
    pub withdrawn: WithdrawnCredential,
    /// Its principal afterwards.
    pub standing: PrincipalStanding,
    /// Writes the withdrawal raced, in admission order.
    pub writes: Vec<RacedWrite>,
}

impl CredentialRevokeResult {
    /// `withdrawn`, leaving `standing`, having raced `writes`.
    #[must_use]
    pub const fn new(
        withdrawn: WithdrawnCredential,
        standing: PrincipalStanding,
        writes: Vec<RacedWrite>,
    ) -> Self {
        Self {
            withdrawn,
            standing,
            writes,
        }
    }
}

/// The replacement a rotation issued.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct IssuedReplacement {
    /// The new credential.
    pub credential: CredentialId,
    /// Its first epoch.
    pub epoch: CredentialEpoch,
    /// When it stops being valid.
    pub expires_at_ms: TimestampMs,
    /// The text to present in `hello`.
    pub bearer: BearerText,
}

impl IssuedReplacement {
    /// `credential` at `epoch` until `expires_at_ms`, presented as `bearer`.
    #[must_use]
    pub fn new(
        (credential, epoch): (CredentialId, CredentialEpoch),
        expires_at_ms: TimestampMs,
        bearer: BearerText,
    ) -> Self {
        Self {
            credential,
            epoch,
            expires_at_ms,
            bearer,
        }
    }
}

/// Result of `credential.rotate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct CredentialRotateResult {
    /// The replacement.
    pub issued: IssuedReplacement,
    /// The replaced credential, withdrawn in the same step.
    pub replaced: CredentialRevokeResult,
}

impl CredentialRotateResult {
    /// `issued` in place of `replaced`.
    #[must_use]
    pub const fn new(issued: IssuedReplacement, replaced: CredentialRevokeResult) -> Self {
        Self { issued, replaced }
    }
}

/// Methods of the `credential` namespace.
pub(crate) const METHODS: &[MethodSpec] = &[
    method! {"credential.rotate", Mutation, Lookup, CredentialRotateParams => CredentialRotateResult,
    "Replace a credential and withdraw the old one in one step, guarded by the principal epoch.",
    [Forbidden, VersionConflict, CredentialRevoked, CredentialExpired, LimitExceeded]},
    method! {"credential.revoke", Mutation, Lookup, CredentialRevokeParams => CredentialRevokeResult,
    "Withdraw a credential; its queued writes are rechecked before any byte is written.",
    [Forbidden]},
];

#[cfg(test)]
mod tests;
