//! Unix service lifetime and endpoint discovery records (plan todo 51).
//!
//! A [`DiscoveryRecord`] names where a holder service listens and which process bound
//! it. It is never liveness: [`Observed`] reports what one observation proved, and an
//! observation never creates, starts or registers anything.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::schema::ids::{ProcessPid, ServiceLabel, StartToken};
use crate::schema::limits::TransportKind;
use crate::schema::methods::service::ServiceManager;

mod status;
#[cfg(test)]
mod tests;

pub use status::{EndedTerminal, Observed, Prerequisite, ServiceStatus, StaleReason, StopReceipt};

/// Format marker of a discovery record; any other marker is refused, never guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DiscoveryFormat {
    /// The first record format.
    #[serde(rename = "ovis-terminal-holder-endpoint/1")]
    V1,
}

/// The process that bound an endpoint: a PID plus the OS start token that rules out reuse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct BootProof {
    /// Process identifier of the holder.
    pub pid: ProcessPid,
    /// Start token of that process.
    pub start_token: StartToken,
}

impl BootProof {
    /// Binds a PID to its start token.
    #[must_use]
    pub const fn new(pid: ProcessPid, start_token: StartToken) -> Self {
        Self { pid, start_token }
    }
}

/// Where a holder service listens and which process bound it; not proof that it lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DiscoveryRecord {
    /// Record format.
    pub format: DiscoveryFormat,
    /// Service label the holder runs under.
    pub label: ServiceLabel,
    /// Manager that launched it; `None` for a detached holder no manager owns.
    pub manager: Option<ServiceManager>,
    /// Transport of the address.
    pub transport: TransportKind,
    /// Absolute socket path.
    pub address: String,
    /// The process that bound the address.
    pub boot_proof: BootProof,
}

impl DiscoveryRecord {
    /// A Unix-socket record with no manager; see [`Self::with_manager`].
    #[must_use]
    pub const fn new(label: ServiceLabel, address: String, boot_proof: BootProof) -> Self {
        Self {
            format: DiscoveryFormat::V1,
            label,
            manager: None,
            transport: TransportKind::Unix,
            address,
            boot_proof,
        }
    }

    /// Names the manager that launched the holder.
    #[must_use]
    pub const fn with_manager(mut self, manager: Option<ServiceManager>) -> Self {
        self.manager = manager;
        self
    }

    /// Parses a stored record.
    ///
    /// # Errors
    ///
    /// [`DiscoveryError::Unsupported`] for another format marker and
    /// [`DiscoveryError::Malformed`] for anything else that is not this exact shape.
    pub fn parse(bytes: &[u8]) -> Result<Self, DiscoveryError> {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(DiscoveryError::Malformed)?;
        let found = value.get("format").and_then(serde_json::Value::as_str);
        if found != Some("ovis-terminal-holder-endpoint/1") {
            return Err(DiscoveryError::Unsupported {
                found: found.map(str::to_owned),
            });
        }
        serde_json::from_value(value).map_err(DiscoveryError::Malformed)
    }
}

/// A stored discovery record that cannot be used; it is reported, never replaced by a default.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DiscoveryError {
    /// The record is not JSON of the published shape.
    #[error("the discovery record does not have the published shape")]
    Malformed(#[source] serde_json::Error),
    /// The record carries a format this build does not read.
    #[error("the discovery record format {found:?} is not readable by this build")]
    Unsupported {
        /// The marker found, if any.
        found: Option<String>,
    },
}

impl DiscoveryError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Malformed(_) => "descriptor_malformed",
            Self::Unsupported { .. } => "descriptor_unsupported",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Malformed(_) => {
                "inspect the record file; `service start` refuses to overwrite it"
            }
            Self::Unsupported { .. } => {
                "use the build that wrote the record, or stop that holder first"
            }
        }
    }
}
