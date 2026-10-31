//! `ovis-terminal service ...` on Windows: explicit holder service verbs, separate from
//! inspection.

use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use ovis_terminal_protocol::domain::windows_service::HolderPlacement;

/// Every service verb; only `status` is read-only, and it never installs or starts.
#[derive(Debug, Subcommand)]
#[non_exhaustive]
pub enum ServiceCommand {
    /// Report running, stopped, unreachable or `stale_endpoint` with the manager's view.
    Status(HostArgs),
    /// Install the holder as a Windows service running as `--account`; nothing starts.
    Install(InstallArgs),
    /// Delete a stopped service this runtime installed for the owner.
    Uninstall(HostArgs),
    /// Start the installed service and wait until its holder serves its pipe.
    Start(HostArgs),
    /// Stop the service; its holder ends every terminal it owns.
    Stop(HostArgs),
    /// Run the holder as the service the manager started; what `start` makes it run.
    #[command(hide = true)]
    Run(RunArgs),
}

/// The session addressed and where its state lives.
#[derive(Debug, Args)]
#[non_exhaustive]
pub struct HostArgs {
    /// Session name: 1 to 32 ASCII letters, digits, `-` or `_`.
    #[arg(long, default_value = "default")]
    pub session: String,
    /// State directory; defaults to `%LOCALAPPDATA%\ovis-terminal\<session>`.
    #[arg(long)]
    pub state_dir: Option<PathBuf>,
    /// SID of the account that owns the service; defaults to the calling account.
    #[arg(long)]
    pub owner_sid: Option<String>,
}

/// Installing names the account; the password is read from
/// `OVIS_TERMINAL_SERVICE_PASSWORD`, never from the command line.
#[derive(Debug, Args)]
#[non_exhaustive]
pub struct InstallArgs {
    /// The session addressed.
    #[command(flatten)]
    pub host: HostArgs,
    /// Account the service runs as, `.\name` or `DOMAIN\name`.
    #[arg(long)]
    pub account: String,
    /// Which holder the host runs.
    #[arg(long, value_enum, default_value = "shared", hide = true)]
    pub placement: Placement,
}

/// The service process's own arguments.
#[derive(Debug, Args)]
#[non_exhaustive]
pub struct RunArgs {
    /// The session addressed.
    #[command(flatten)]
    pub host: HostArgs,
    /// Which holder the host runs.
    #[arg(long, value_enum)]
    pub placement: Placement,
}

/// Holder placement the host selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[non_exhaustive]
pub enum Placement {
    /// The session's shared holder.
    Shared,
    /// A holder that owns at most one terminal.
    Dedicated,
}

impl Placement {
    /// The protocol placement.
    #[must_use]
    pub const fn protocol(self) -> HolderPlacement {
        match self {
            Self::Shared => HolderPlacement::Shared,
            Self::Dedicated => HolderPlacement::Dedicated,
        }
    }
}
