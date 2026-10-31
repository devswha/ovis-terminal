//! Windows holder service records (plan todo 54).
//!
//! A Windows holder service publishes a [`HostRecord`]. Its [`DiscoveryRecord`] names the
//! named pipe the holder serves, a connectable address rather than a marker file, and the
//! record says which [`HolderPlacement`] the host selected. [`WindowsServiceStatus`] joins
//! one observation with the service control manager's view; observing never installs,
//! starts or repairs anything.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::unix_service::{BootProof, DiscoveryError, DiscoveryRecord, Observed};
use crate::schema::ids::{ProcessPid, ServiceLabel};
use crate::schema::limits::TransportKind;
use crate::schema::methods::service::{EndpointDescriptor, ServiceManager, ServiceRecord};
use crate::schema::scalar::OpenEnum;

/// Longest pipe name Windows accepts, prefix included.
const MAX_PIPE_NAME: usize = 256;
/// Built-in accounts a holder must never run as: their programs would not be the user's.
const PRIVILEGED_ACCOUNTS: [&str; 4] = ["localsystem", "localservice", "networkservice", "system"];

/// A local named pipe such as `\\.\pipe\ovis-terminal-holder-S-1-5-21-1-default`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PipeName(String);

impl PipeName {
    /// Parses a local pipe name: the `\\.\pipe\` prefix, then 1 or more characters without
    /// a backslash or NUL, 256 characters at most in total.
    ///
    /// # Errors
    ///
    /// [`WindowsServiceRecordError::InvalidPipeName`] for anything else.
    pub fn parse(text: &str) -> Result<Self, WindowsServiceRecordError> {
        let name = text
            .strip_prefix(r"\\.\pipe\")
            .ok_or(WindowsServiceRecordError::InvalidPipeName)?;
        let valid = !name.is_empty()
            && text.chars().count() <= MAX_PIPE_NAME
            && !name.contains(['\\', '\0']);
        if !valid {
            return Err(WindowsServiceRecordError::InvalidPipeName);
        }
        Ok(Self(text.to_owned()))
    }

    /// The full pipe name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PipeName {
    type Error = WindowsServiceRecordError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl From<PipeName> for String {
    fn from(name: PipeName) -> Self {
        name.0
    }
}

/// The account a holder service runs as: `.\name` or `DOMAIN\name`, never a built-in
/// service account, so its programs run as the user who owns the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceAccount(String);

impl ServiceAccount {
    /// Parses an account name.
    ///
    /// # Errors
    ///
    /// [`WindowsServiceRecordError::InvalidAccount`] for another shape and
    /// [`WindowsServiceRecordError::PrivilegedAccount`] for a built-in service account.
    pub fn parse(text: &str) -> Result<Self, WindowsServiceRecordError> {
        let (domain, name) = text
            .split_once('\\')
            .ok_or(WindowsServiceRecordError::InvalidAccount)?;
        let bad = |part: &str| part.is_empty() || part.contains(['\\', '\0', '"', '/']);
        if bad(domain) || bad(name) {
            return Err(WindowsServiceRecordError::InvalidAccount);
        }
        let lowered = name.to_ascii_lowercase();
        if domain.eq_ignore_ascii_case("NT AUTHORITY")
            || PRIVILEGED_ACCOUNTS.contains(&lowered.as_str())
        {
            return Err(WindowsServiceRecordError::PrivilegedAccount);
        }
        Ok(Self(text.to_owned()))
    }

    /// The account name as the service control manager takes it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Which holder the host runs; the host selects it, no client launch field does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HolderPlacement {
    /// The session's shared holder: every terminal of the session.
    Shared,
    /// A dedicated holder that owns at most one terminal.
    Dedicated,
}

impl HolderPlacement {
    /// The wire token, as `--placement` takes it.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::Dedicated => "dedicated",
        }
    }
}

/// The state the service control manager reports for a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ScmState {
    /// `SERVICE_STOPPED`.
    Stopped,
    /// `SERVICE_START_PENDING`.
    StartPending,
    /// `SERVICE_STOP_PENDING`.
    StopPending,
    /// `SERVICE_RUNNING`.
    Running,
    /// `SERVICE_CONTINUE_PENDING`.
    ContinuePending,
    /// `SERVICE_PAUSE_PENDING`.
    PausePending,
    /// `SERVICE_PAUSED`.
    Paused,
}

impl ScmState {
    /// The state for a raw `dwCurrentState`; an undocumented value is `None`.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Option<Self> {
        Some(match raw {
            1 => Self::Stopped,
            2 => Self::StartPending,
            3 => Self::StopPending,
            4 => Self::Running,
            5 => Self::ContinuePending,
            6 => Self::PausePending,
            7 => Self::Paused,
            _ => return None,
        })
    }
}

/// Format marker of a host record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum HostFormat {
    /// The first host record format.
    #[serde(rename = "ovis-terminal-windows-host/1")]
    V1,
}

/// What a running Windows holder service published: its pipe endpoint and placement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct HostRecord {
    /// Record format.
    pub format: HostFormat,
    /// The pipe the holder serves and the process that created it.
    pub endpoint: DiscoveryRecord,
    /// The placement the host selected.
    pub placement: HolderPlacement,
}

impl HostRecord {
    /// A record of a holder serving `pipe` under `label`, bound by `proof`.
    #[must_use]
    pub fn new(label: ServiceLabel, pipe: &PipeName, proof: BootProof) -> Self {
        let endpoint = DiscoveryRecord {
            transport: TransportKind::Pipe,
            ..DiscoveryRecord::new(label, pipe.as_str().to_owned(), proof)
        };
        Self {
            format: HostFormat::V1,
            endpoint: endpoint.with_manager(Some(ServiceManager::WindowsService)),
            placement: HolderPlacement::Shared,
        }
    }

    /// Records the placement the host selected.
    #[must_use]
    pub const fn with_placement(mut self, placement: HolderPlacement) -> Self {
        self.placement = placement;
        self
    }

    /// The pipe the record names.
    ///
    /// # Errors
    ///
    /// [`WindowsServiceRecordError::InvalidPipeName`] when the address is not a pipe.
    pub fn pipe(&self) -> Result<PipeName, WindowsServiceRecordError> {
        if self.endpoint.transport != TransportKind::Pipe {
            return Err(WindowsServiceRecordError::InvalidPipeName);
        }
        PipeName::parse(&self.endpoint.address)
    }

    /// Parses a stored record; a Unix-socket endpoint is malformed here.
    ///
    /// # Errors
    ///
    /// [`DiscoveryError::Unsupported`] for another format marker and
    /// [`DiscoveryError::Malformed`] for any other shape or a non-pipe address.
    pub fn parse(bytes: &[u8]) -> Result<Self, DiscoveryError> {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(DiscoveryError::Malformed)?;
        let found = value.get("format").and_then(serde_json::Value::as_str);
        if found != Some("ovis-terminal-windows-host/1") {
            return Err(DiscoveryError::Unsupported {
                found: found.map(str::to_owned),
            });
        }
        let record: Self = serde_json::from_value(value).map_err(DiscoveryError::Malformed)?;
        record.pipe().map_err(|error| {
            DiscoveryError::Malformed(serde::de::Error::custom(error.to_string()))
        })?;
        Ok(record)
    }
}

impl ServiceRecord {
    /// A Windows service registration of `label` whose holder serves `pipe`.
    #[must_use]
    pub fn pipe_registration(label: ServiceLabel, pipe: &PipeName) -> Self {
        Self {
            label,
            manager: OpenEnum::Known(ServiceManager::WindowsService),
            endpoint: EndpointDescriptor {
                transport: OpenEnum::Known(TransportKind::Pipe),
                address: pipe.as_str().to_owned(),
                server_boot_id: None,
            },
            missing_prerequisites: Vec::new(),
        }
    }
}

/// The service control manager's view of an installed service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Installation {
    /// Current state.
    pub state: ScmState,
    /// Service process, while one runs.
    pub pid: Option<ProcessPid>,
}

impl Installation {
    /// An installation in `state`, with its process when one runs.
    #[must_use]
    pub const fn new(state: ScmState, pid: Option<ProcessPid>) -> Self {
        Self { state, pid }
    }
}

/// `service status` on Windows: one observation plus the manager's view; never a mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WindowsServiceStatus {
    /// Service label observed.
    pub label: ServiceLabel,
    /// What the observation proved.
    #[serde(flatten)]
    pub observed: Observed,
    /// The installed service, `None` when this runtime installed none.
    pub installed: Option<Installation>,
    /// The placement the running holder published.
    pub placement: Option<HolderPlacement>,
}

impl WindowsServiceStatus {
    /// A status of `label`.
    #[must_use]
    pub const fn new(label: ServiceLabel, observed: Observed) -> Self {
        Self {
            label,
            observed,
            installed: None,
            placement: None,
        }
    }

    /// Adds the manager's view and the published placement.
    #[must_use]
    pub const fn with_host(
        mut self,
        installed: Option<Installation>,
        placement: Option<HolderPlacement>,
    ) -> Self {
        self.installed = installed;
        self.placement = placement;
        self
    }
}

/// A Windows service value that cannot be used; never replaced by a default.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WindowsServiceRecordError {
    /// The text is not a local pipe name.
    #[error("the address is not a local named pipe of at most 256 characters")]
    InvalidPipeName,
    /// The account is not `.\name` or `DOMAIN\name`.
    #[error("the account is not of the form DOMAIN\\name")]
    InvalidAccount,
    /// The account is a built-in service account.
    #[error("a holder service must not run as a built-in service account")]
    PrivilegedAccount,
}

impl WindowsServiceRecordError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::InvalidPipeName => "invalid_pipe_name",
            Self::InvalidAccount => "invalid_account",
            Self::PrivilegedAccount => "privileged_account",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::InvalidPipeName => "name a pipe as \\\\.\\pipe\\<name> without backslashes",
            Self::InvalidAccount => "name the account as .\\user or DOMAIN\\user",
            Self::PrivilegedAccount => "install the service as the user who owns the runtime",
        }
    }
}

#[cfg(test)]
mod tests;
