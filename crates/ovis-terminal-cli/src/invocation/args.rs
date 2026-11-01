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
    #[cfg(any(unix, windows))]
    #[command(subcommand)]
    Service(crate::service::ServiceCommand),
    /// Run the control server in the foreground: endpoint, stores, holder and every routed method.
    #[cfg(unix)]
    Serve(ServeArgs),
    /// Run the session holder in the foreground, owned by no service manager.
    #[cfg(unix)]
    Hold(crate::service::unix::SessionArgs),
}

/// `ovis-terminal serve [--root <dir>] [--stdin-lifetime]`.
#[cfg(unix)]
#[derive(Debug, Args)]
pub(super) struct ServeArgs {
    /// Keep the socket at `<root>/run/s.sock` and state in `<root>/state`.
    #[arg(long)]
    pub(super) root: Option<std::path::PathBuf>,
    /// Also stop when standard input reaches end of file.
    #[arg(long)]
    pub(super) stdin_lifetime: bool,
    /// Also serve the protocol over WebSocket on this loopback port (0 picks a free one).
    #[arg(long)]
    pub(super) websocket_port: Option<u16>,
    /// QA seam: how long the runtime waits for a request, with the control released,
    /// before it looks for holder messages again (milliseconds; 5 without it). Accepted
    /// only with a `--root` that holds a `qa-fixture` file.
    #[arg(long)]
    pub(super) qa_holder_poll_ms: Option<u64>,
    /// QA seam: before the next `terminal.run`, the runtime, holding the control, writes
    /// `held` to this FIFO, then waits until a writer opens it again. Accepted only with a
    /// `--root` that holds a `qa-fixture` file.
    #[arg(long)]
    pub(super) qa_actor_hold: Option<std::path::PathBuf>,
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
    /// Attach this terminal to a running terminal; Ctrl-\ detaches.
    #[cfg(unix)]
    Attach(AttachArgs),
}

/// `ovis-terminal terminal attach (--target <address> | --terminal-id <id> | --name <name>)`.
#[cfg(unix)]
#[derive(Debug, Args)]
pub(super) struct AttachArgs {
    /// The terminal to attach to.
    #[command(flatten)]
    pub(super) target: AttachTarget,
    /// Watch only: needs no TTY and never reads or forwards input.
    #[arg(long)]
    pub(super) observe: bool,
    /// Client label the lease is recorded under.
    #[arg(long, default_value = "cli")]
    pub(super) client_id: String,
}

/// Exactly one explicit way to name the attached terminal.
#[cfg(unix)]
#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
pub(super) struct AttachTarget {
    /// Terminal address such as `t1` or `machine/t1`.
    #[arg(long)]
    pub(super) target: Option<String>,
    /// Stable terminal identity.
    #[arg(long)]
    pub(super) terminal_id: Option<String>,
    /// Unique live terminal name.
    #[arg(long)]
    pub(super) name: Option<String>,
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
