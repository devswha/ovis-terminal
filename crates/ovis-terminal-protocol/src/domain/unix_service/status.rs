//! What an observation proved, logout prerequisites, and stop receipts.

use serde::{Deserialize, Serialize};

use super::{BootProof, DiscoveryRecord};
use crate::schema::ids::{ServiceLabel, TerminalId};
use crate::schema::limits::TransportKind;
use crate::schema::methods::server::ServerState;
use crate::schema::methods::service::{
    EndpointDescriptor, ServiceDescribeResult, ServiceManager, ServiceRecord,
};
use crate::schema::scalar::{Incarnation, OpenEnum};

/// Why an endpoint names a holder that is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StaleReason {
    /// The recorded PID names no process.
    ProcessGone,
    /// The recorded PID now names another process (start token differs).
    ProcessReplaced,
    /// A socket exists, no record names it and nothing accepts on it.
    SocketRefused,
    /// The endpoint path holds something that is not a socket.
    NotASocket,
}

/// What one observation proved; the tag is the [`ServerState`] token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Observed {
    /// The recorded process lives with its start token and accepts on the address.
    Running {
        /// The record observed.
        record: DiscoveryRecord,
    },
    /// No record and nothing at the address.
    Stopped,
    /// The holder lives (or something accepts) but its endpoint does not answer.
    Unreachable {
        /// The record, when one exists.
        record: Option<DiscoveryRecord>,
    },
    /// The endpoint names a holder that is gone.
    StaleEndpoint {
        /// The record, when one exists.
        record: Option<DiscoveryRecord>,
        /// Why it is stale.
        reason: StaleReason,
    },
}

impl Observed {
    /// The protocol state token of this observation.
    #[must_use]
    pub const fn state(&self) -> ServerState {
        match self {
            Self::Running { .. } => ServerState::Running,
            Self::Stopped => ServerState::Stopped,
            Self::Unreachable { .. } => ServerState::Unreachable,
            Self::StaleEndpoint { .. } => ServerState::StaleEndpoint,
        }
    }
}

/// A condition a holder needs to outlive its user's logout that is not met.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Prerequisite {
    /// No service manager owns the holder: a detached process is not logout-safe.
    ServiceManager,
    /// The systemd user manager does not answer.
    SystemdUserManager,
    /// The user has no lingering, so systemd ends user services at logout.
    UserLinger,
    /// The launchd user domain does not answer.
    LaunchdUserDomain,
}

impl Prerequisite {
    /// The wire token, as `service.describe` lists it.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::ServiceManager => "service_manager",
            Self::SystemdUserManager => "systemd_user_manager",
            Self::UserLinger => "user_linger",
            Self::LaunchdUserDomain => "launchd_user_domain",
        }
    }
}

impl ServiceRecord {
    /// A registration of `label` under `manager` whose holder binds the Unix `address`.
    ///
    /// The descriptor carries no boot identity: a registration is not a running holder.
    #[must_use]
    pub const fn registration(
        label: ServiceLabel,
        manager: ServiceManager,
        address: String,
    ) -> Self {
        Self {
            label,
            manager: OpenEnum::Known(manager),
            endpoint: EndpointDescriptor {
                transport: OpenEnum::Known(TransportKind::Unix),
                address,
                server_boot_id: None,
            },
            missing_prerequisites: Vec::new(),
        }
    }

    /// Lists the unmet logout-survival prerequisites.
    #[must_use]
    pub fn with_missing(mut self, missing: &[Prerequisite]) -> Self {
        self.missing_prerequisites = missing.iter().map(|item| item.token().to_owned()).collect();
        self
    }
}

impl ServiceDescribeResult {
    /// Every registration described.
    #[must_use]
    pub const fn new(services: Vec<ServiceRecord>) -> Self {
        Self { services }
    }
}

/// `service status`: one observation plus the registration facts; never a mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ServiceStatus {
    /// Service label observed.
    pub label: ServiceLabel,
    /// What the observation proved.
    #[serde(flatten)]
    pub observed: Observed,
    /// Manager holding a registration for the label, if any.
    pub registered: Option<ServiceManager>,
    /// Unmet conditions for the holder to outlive a logout.
    pub missing_prerequisites: Vec<Prerequisite>,
}

impl ServiceStatus {
    /// A status with no registration and no missing prerequisite.
    #[must_use]
    pub const fn new(label: ServiceLabel, observed: Observed) -> Self {
        Self {
            label,
            observed,
            registered: None,
            missing_prerequisites: Vec::new(),
        }
    }

    /// Adds the registration facts.
    #[must_use]
    pub fn with_registration(
        mut self,
        registered: Option<ServiceManager>,
        missing: Vec<Prerequisite>,
    ) -> Self {
        self.registered = registered;
        self.missing_prerequisites = missing;
        self
    }
}

/// One terminal a holder stop ended, with its process incarnation when one ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EndedTerminal {
    /// Terminal identity.
    pub terminal_id: TerminalId,
    /// Terminal incarnation.
    pub terminal_incarnation: Incarnation,
    /// Process incarnation, when a process was spawned.
    pub process_incarnation: Option<Incarnation>,
    /// The process, when one was still recorded.
    pub process: Option<BootProof>,
}

impl EndedTerminal {
    /// A terminal record without a process.
    #[must_use]
    pub const fn new(terminal_id: TerminalId, terminal_incarnation: Incarnation) -> Self {
        Self {
            terminal_id,
            terminal_incarnation,
            process_incarnation: None,
            process: None,
        }
    }

    /// Adds the process the terminal held.
    #[must_use]
    pub fn with_process(mut self, incarnation: Incarnation, process: BootProof) -> Self {
        self.process_incarnation = Some(incarnation);
        self.process = Some(process);
        self
    }
}

/// `service stop`: the holder incarnation that ended and every terminal it ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct StopReceipt {
    /// Service label stopped.
    pub label: ServiceLabel,
    /// The holder process that ended.
    pub ended_holder: BootProof,
    /// Terminals the holder still owned when it ended.
    pub ended_terminals: Vec<EndedTerminal>,
}

impl StopReceipt {
    /// A receipt for one ended holder.
    #[must_use]
    pub const fn new(
        label: ServiceLabel,
        holder: BootProof,
        terminals: Vec<EndedTerminal>,
    ) -> Self {
        Self {
            label,
            ended_holder: holder,
            ended_terminals: terminals,
        }
    }
}
