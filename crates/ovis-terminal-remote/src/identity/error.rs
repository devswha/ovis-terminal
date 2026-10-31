//! Failures of machine identity, naming and trust.

use ovis_terminal_protocol::schema::ids::{MachineId, MachineName};

/// Why a machine could not be named, resolved, trusted or recorded.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IdentityError {
    /// The name violates the naming rules.
    #[error("the machine name is not 1 to 63 lowercase letters, digits, '-', '_' or '.'")]
    InvalidName,
    /// The name or alias already names a machine, or another add holds it.
    #[error("{} already names a machine", name.as_str())]
    DuplicateName {
        /// The taken name.
        name: MachineName,
    },
    /// The machine the runtime proved is already recorded.
    #[error("{} is already added", machine.as_str())]
    MachineExists {
        /// The proven machine.
        machine: MachineId,
    },
    /// No machine has this identity, name or alias.
    #[error("no machine answers to this reference")]
    NotFound,
    /// The add named no expected machine identity.
    #[error("the add names no expected machine identity")]
    TrustRequired,
    /// The runtime proved another machine identity than the trusted one.
    #[error("the runtime proved {}, not the expected {}", actual.as_str(), expected.as_str())]
    TrustMismatch {
        /// The configured identity.
        expected: MachineId,
        /// The proven identity.
        actual: MachineId,
    },
    /// The hello result carries no boot proof.
    #[error("the remote runtime answered the challenge without a boot proof")]
    ProofMissing,
    /// The proof names an identity other than the hello result it came with.
    #[error("the boot proof and the hello result name different runtimes")]
    ProofInconsistent,
    /// The proof answers another challenge, such as one cached from an earlier connection.
    #[error("the boot proof answers another challenge")]
    BootProofStale,
    /// The proof was not keyed by the presented credential.
    #[error("the boot proof was not made with the presented credential")]
    ProofForged,
}

impl IdentityError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::InvalidName => "invalid_name",
            Self::DuplicateName { .. } => "duplicate_name",
            Self::MachineExists { .. } => "machine_exists",
            Self::NotFound => "not_found",
            Self::TrustRequired => "trust_required",
            Self::TrustMismatch { .. } => "trust_mismatch",
            Self::ProofMissing | Self::ProofInconsistent => "remote_incompatible",
            Self::BootProofStale => "boot_proof_stale",
            Self::ProofForged => "auth_failed",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::InvalidName => "use 1 to 63 lowercase letters, digits, '-', '_' or '.'",
            Self::DuplicateName { .. } => "choose a name that no machine or alias uses",
            Self::MachineExists { .. } => "rename the existing machine instead of adding it again",
            Self::NotFound => "list machines to find a live id or name",
            Self::TrustRequired => "pass expected_machine_id from the remote's hello",
            Self::TrustMismatch { .. } => "verify the transport reaches the expected runtime",
            Self::ProofMissing | Self::ProofInconsistent => {
                "update the remote runtime to one that answers boot challenges"
            }
            Self::BootProofStale => "reconnect; a proof is valid for one challenge only",
            Self::ProofForged => "issue a fresh credential on the remote runtime",
        }
    }
}
