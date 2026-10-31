//! The command-line grammar. Parsing reads only the argument list.

use clap::{Args, Parser, Subcommand, ValueEnum};

/// `ovis-terminal [--json] (--skill | <command>)`.
#[derive(Debug, Parser)]
#[command(
    name = crate::BINARY_NAME,
    version,
    about = "Agent-facing client of the ovis-terminal runtime.",
    disable_help_subcommand = true
)]
pub(super) struct Cli {
    /// Print schema-defined JSON: results on stdout, errors on stderr. `OVIS_JSON=1` does the same.
    #[arg(long, global = true)]
    pub(super) json: bool,
    /// Print the agent manual of this exact build and exit.
    #[arg(long)]
    pub(super) skill: bool,
    #[command(subcommand)]
    pub(super) command: Option<Command>,
}

/// Every command; none of them starts, installs or restarts a server.
#[derive(Debug, Subcommand)]
pub(super) enum Command {
    /// Bridge stdin and stdout to the local endpoint; stdout carries protocol bytes only.
    Connect,
    /// Print the OpenRPC document or the command-line contract of this build.
    Schema {
        /// Which contract to print.
        #[arg(value_enum, default_value_t = SchemaPart::Openrpc)]
        part: SchemaPart,
    },
    /// Describe the endpoint this invocation resolves to, without touching it.
    Endpoint,
    /// Control-server facts.
    #[command(subcommand)]
    Server(ServerCommand),
    /// Terminal records.
    #[command(subcommand)]
    Terminal(TerminalCommand),
    /// Operation records.
    #[command(subcommand)]
    Op(OpCommand),
    /// Send one published method with JSON params.
    Call(CallArgs),
    /// The session holder's OS service: status, install, uninstall, start, stop.
    #[cfg(unix)]
    #[command(subcommand)]
    Service(crate::service::unix::ServiceCommand),
}

/// The two published contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(super) enum SchemaPart {
    /// The protocol's OpenRPC document.
    Openrpc,
    /// Commands, flags, environment and exit codes of this binary.
    Cli,
}

/// `ovis-terminal server ...`.
#[derive(Debug, Subcommand)]
pub(super) enum ServerCommand {
    /// Report running, stopped, unreachable or `stale_endpoint`; never starts a server.
    Status,
}

/// `ovis-terminal terminal ...`.
#[derive(Debug, Subcommand)]
pub(super) enum TerminalCommand {
    /// List terminals.
    List,
    /// Read one terminal record; without a target, answer `needs_target` with candidates.
    Get(TargetArgs),
}

/// At most one explicit way to name a terminal; there is no focus fallback.
#[derive(Debug, Args)]
#[group(multiple = false)]
pub(super) struct TargetArgs {
    /// Stable terminal identity.
    #[arg(long)]
    pub(super) terminal_id: Option<String>,
    /// Terminal address such as `t7` or `machine/t7`.
    #[arg(long)]
    pub(super) address: Option<String>,
    /// Unique live terminal name.
    #[arg(long)]
    pub(super) name: Option<String>,
}

/// `ovis-terminal op ...`.
#[derive(Debug, Subcommand)]
pub(super) enum OpCommand {
    /// Inspect an operation.
    Get(OpArgs),
    /// Cancel an operation before it commits.
    Cancel(OpArgs),
}

/// The operation an `op` command names.
#[derive(Debug, Args)]
pub(super) struct OpArgs {
    /// Operation identity.
    #[arg(long)]
    pub(super) op_id: String,
}

/// `ovis-terminal call <method> [--params <json>]`.
#[derive(Debug, Args)]
pub(super) struct CallArgs {
    /// Published method name, for example `terminal.list`.
    pub(super) method: String,
    /// Params object as JSON text.
    #[arg(long, default_value = "{}")]
    pub(super) params: String,
}
