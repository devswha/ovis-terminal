//! `ovis-terminal service ...`: explicit holder service verbs, separate from inspection.

use clap::{Args, Subcommand, ValueEnum};
use ovis_terminal_protocol::schema::methods::service::ServiceManager;

/// Every service verb; only `status` is read-only, and it never starts or registers.
#[derive(Debug, Subcommand)]
#[non_exhaustive]
pub enum ServiceCommand {
    /// Report running, stopped, unreachable or `stale_endpoint`; never starts or registers.
    Status(SessionArgs),
    /// Write and load the service definition for a manager; nothing starts.
    Install(ManagedArgs),
    /// Remove a definition this runtime wrote; the holder must be stopped first.
    Uninstall(ManagedArgs),
    /// Start the session holder under a manager, or detached without logout survival.
    Start(StartArgs),
    /// Stop the session holder; the receipt lists every terminal and process it ended.
    Stop(SessionArgs),
    /// Run the session holder in this process; what a manager or `start` executes.
    #[command(hide = true)]
    Run(StartArgs),
}

/// The session whose holder is addressed.
#[derive(Debug, Args)]
#[non_exhaustive]
pub struct SessionArgs {
    /// Session name: 1 to 32 ASCII letters, digits, `-` or `_`.
    #[arg(long, default_value = "default")]
    pub session: String,
}

/// A registration verb names its manager; there is no default.
#[derive(Debug, Args)]
#[non_exhaustive]
pub struct ManagedArgs {
    /// The session addressed.
    #[command(flatten)]
    pub session: SessionArgs,
    /// Service manager that owns the registration.
    #[arg(long, value_enum)]
    pub manager: Manager,
}

/// Starting names its placement; there is no default.
#[derive(Debug, Args)]
#[non_exhaustive]
pub struct StartArgs {
    /// The session addressed.
    #[command(flatten)]
    pub session: SessionArgs,
    /// Who owns the holder: a service manager, or nobody (`detached`).
    #[arg(long, value_enum)]
    pub manager: Placement,
}

/// A service manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[non_exhaustive]
pub enum Manager {
    /// A systemd user service (Linux).
    SystemdUser,
    /// A launchd agent (macOS).
    Launchd,
}

impl Manager {
    /// The protocol name of the manager.
    #[must_use]
    pub const fn protocol(self) -> ServiceManager {
        match self {
            Self::SystemdUser => ServiceManager::SystemdUser,
            Self::Launchd => ServiceManager::Launchd,
        }
    }
}

/// Where a started holder lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[non_exhaustive]
pub enum Placement {
    /// A systemd user service (Linux).
    SystemdUser,
    /// A launchd agent (macOS).
    Launchd,
    /// Its own session, owned by no manager; it does not survive a logout that ends
    /// every user process.
    Detached,
}

impl Placement {
    /// The manager owning the holder, `None` when detached.
    #[must_use]
    pub const fn manager(self) -> Option<Manager> {
        match self {
            Self::SystemdUser => Some(Manager::SystemdUser),
            Self::Launchd => Some(Manager::Launchd),
            Self::Detached => None,
        }
    }
}
