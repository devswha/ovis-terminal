//! Fresh boot challenges, proof verification and the explicit trust anchor.

use ovis_terminal_authz::Attestation;
use ovis_terminal_protocol::domain::machine_identity::BootChallenge;
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::methods::server::HelloResult;

use super::{Credential, IdentityError};

/// 32 bytes from two operating-system random UUIDs, never reused across connections.
#[must_use]
pub fn fresh_challenge() -> BootChallenge {
    let (high, low) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
    let mut bytes = [0_u8; 32];
    for (slot, byte) in bytes
        .iter_mut()
        .zip(high.as_bytes().iter().chain(low.as_bytes()))
    {
        *slot = *byte;
    }
    BootChallenge::from_bytes(bytes)
}

/// A hello result whose boot proof answered this connection's challenge with the presented
/// credential and names the configured machine. Only [`Trust::admit`] builds one.
#[derive(Debug, Clone)]
pub struct Trusted(HelloResult);

impl Trusted {
    /// The authenticated hello result.
    #[must_use]
    pub const fn hello(&self) -> &HelloResult {
        &self.0
    }

    /// The proven machine identity.
    #[must_use]
    pub const fn machine_id(&self) -> &MachineId {
        &self.0.machine_id
    }
}

/// Explicit trust in one remote runtime: the machine identity it must prove and the
/// credential it issued, which keys its boot proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trust {
    expected: MachineId,
    credential: Credential,
}

impl Trust {
    /// Explicit trust; there is no trust on first use.
    ///
    /// # Errors
    ///
    /// [`IdentityError::TrustRequired`] when no identity is configured.
    pub fn required(
        expected: Option<MachineId>,
        credential: Credential,
    ) -> Result<Self, IdentityError> {
        expected
            .map(|expected| Self {
                expected,
                credential,
            })
            .ok_or(IdentityError::TrustRequired)
    }

    /// The credential presented in the handshake.
    #[must_use]
    pub const fn credential(&self) -> &Credential {
        &self.credential
    }

    /// Verifies that `hello` answers `challenge` with a proof keyed by the credential, then
    /// that the proven machine is the expected one. Nothing is recorded on failure.
    ///
    /// # Errors
    ///
    /// [`IdentityError::ProofMissing`] or [`IdentityError::ProofInconsistent`] for a runtime
    /// that cannot prove its boot, [`IdentityError::BootProofStale`] for a proof of another
    /// challenge, [`IdentityError::ProofForged`] for a proof not keyed by the credential and
    /// [`IdentityError::TrustMismatch`] for another machine.
    pub fn admit(
        &self,
        hello: HelloResult,
        challenge: BootChallenge,
    ) -> Result<Trusted, IdentityError> {
        let proof = hello
            .boot_proof
            .as_ref()
            .ok_or(IdentityError::ProofMissing)?;
        if proof.statement.challenge != challenge {
            return Err(IdentityError::BootProofStale);
        }
        if !proof.statement.describes(&hello) {
            return Err(IdentityError::ProofInconsistent);
        }
        let expected =
            Attestation::of_bearer(self.credential.expose(), &proof.statement.signed_bytes());
        if !expected.matches(&Attestation::from_bytes(proof.mac.to_bytes())) {
            return Err(IdentityError::ProofForged);
        }
        if hello.machine_id != self.expected {
            return Err(IdentityError::TrustMismatch {
                expected: self.expected.clone(),
                actual: hello.machine_id,
            });
        }
        Ok(Trusted(hello))
    }
}

#[cfg(test)]
#[path = "proof/tests.rs"]
mod tests;
