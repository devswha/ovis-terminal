//! The command-line boundary error: everything this binary refuses or fails at itself.

use std::io;
use std::path::PathBuf;

use ovis_terminal_protocol::errors::{BoundaryError, TargetCandidate};
use ovis_terminal_protocol::schema::ParamsError;
use ovis_terminal_server::listen::EndpointError;
use thiserror::Error;

use super::context::OutputMode;

/// Why an invocation failed before or while talking to a server.
#[derive(Debug, Error)]
pub(super) enum CliError {
    /// The argument list does not parse; nothing was sent.
    #[error("the command line does not parse: {detail}")]
    Usage {
        /// Clap's first diagnostic line.
        detail: String,
        /// The parser's error.
        #[source]
        source: clap::Error,
    },
    /// Neither a command nor `--skill` was given.
    #[error("a command or --skill is required")]
    MissingCommand,
    /// `--skill` was combined with a command.
    #[error("--skill takes no command")]
    SkillWithCommand,
    /// `OVIS_JSON` holds something other than `0`, `1` or nothing.
    #[error("OVIS_JSON must be 0, 1 or unset")]
    JsonSwitch {
        /// The mode the error itself is written in.
        mode: OutputMode,
    },
    /// The params do not match the method's closed schema; nothing was sent.
    #[error("the params violate the method schema at {}", .0.field_path())]
    Params(#[source] ParamsError),
    /// `--params` is not a JSON object.
    #[error("--params is not a JSON object")]
    ParamsJson(#[source] serde_json::Error),
    /// A typed verb's flags and `--params` could not be merged.
    #[error(transparent)]
    Command(crate::commands::CommandError),
    /// The method is not in the published catalog, or is `hello`.
    #[error("{method} is not a published method this command can send")]
    UnknownMethod {
        /// The rejected name.
        method: String,
    },
    /// No explicit target was named; the candidates are listed instead of guessing.
    #[error("the command needs an explicit target")]
    NeedsTarget {
        /// Live alternatives, empty when no server answered.
        candidates: Vec<TargetCandidate>,
        /// Whether a server answered the candidate listing.
        listed: bool,
    },
    /// The endpoint could not be resolved or inspected.
    #[error("the endpoint cannot be used")]
    Endpoint(#[source] EndpointError),
    /// No server listens at the endpoint; this binary never starts one.
    #[error("no control server answers at {path}")]
    NotRunning {
        /// The socket path.
        path: PathBuf,
        /// `stopped` or `stale_endpoint`.
        state: &'static str,
    },
    /// The transport to the endpoint failed.
    #[error("{operation} failed")]
    Io {
        /// What was attempted.
        operation: &'static str,
        /// The underlying failure.
        #[source]
        source: io::Error,
    },
    /// Every instance of the Windows named pipe stayed busy for the whole connect bound.
    #[cfg(windows)]
    #[error("every instance of {pipe} stayed busy for {waited_ms} ms")]
    PipeBusy {
        /// The pipe name.
        pipe: String,
        /// The bound waited for a free instance.
        waited_ms: u64,
        /// The last refusal, `ERROR_PIPE_BUSY`.
        #[source]
        source: io::Error,
    },
    /// The server closed the connection before answering.
    #[error("the server closed the connection before answering")]
    Closed,
    /// The server ended the connection for a planned restart before answering.
    #[error("the server restarted before answering")]
    Restarted,
    /// The server sent something that is not a JSON-RPC reply to this request.
    #[error("the server reply is not a JSON-RPC reply")]
    Reply(#[source] BoundaryError),
    /// A success result does not match the published result schema.
    #[error("the server result does not match the published schema")]
    Result(#[source] serde_json::Error),
    /// The server did not settle the request in time.
    #[error("the server did not answer within {deadline_ms} ms")]
    Timeout {
        /// The bound waited for.
        deadline_ms: u64,
    },
    /// This platform has no client transport in this build.
    #[error("this platform has no client transport in this build")]
    UnsupportedTransport,
    /// The embedded OpenRPC document could not be rendered.
    #[error("the OpenRPC document could not be rendered")]
    Schema(#[source] ovis_terminal_protocol::schema::SchemaError),
    /// `--released` without `schema compat`, or `schema compat` without `--released`.
    #[error("--released goes with `schema compat`, and `schema compat` needs it")]
    ReleasedFlag,
    /// The released baseline could not be read whole.
    #[error("cannot read the released baseline {path}")]
    BaselineUnreadable {
        /// The `--released` path.
        path: PathBuf,
        /// The read failure, or the size bound it crossed.
        #[source]
        source: io::Error,
    },
    /// The released baseline is not JSON.
    #[error("the released baseline {path} is not JSON")]
    BaselineJson {
        /// The `--released` path.
        path: PathBuf,
        /// The parse failure.
        #[source]
        source: serde_json::Error,
    },
    /// The released baseline is JSON but not an OpenRPC document the gate can compare.
    #[error("the released baseline {path} is not a comparable OpenRPC document")]
    BaselineSchema {
        /// The `--released` path.
        path: PathBuf,
        /// What the comparison found missing or misplaced.
        #[source]
        source: ovis_terminal_protocol::schema::SchemaError,
    },
    /// Same-generation rules apply only within one protocol generation.
    #[error(
        "the released baseline is generation {released}; this build speaks generation {current}"
    )]
    GenerationMismatch {
        /// `info.x-generation` of the baseline.
        released: String,
        /// The generation of this build.
        current: String,
    },
    /// This build would break a client of the released baseline.
    #[error("this build breaks {} released contract rule(s)", .breaks.len())]
    CompatibilityBreak {
        /// Every break, with its location and rule.
        breaks: Vec<ovis_terminal_protocol::schema::Break>,
    },
    /// A holder service verb failed.
    #[cfg(any(unix, windows))]
    #[error(transparent)]
    Service(crate::service::ServiceError),
    /// A request or result could not be encoded.
    #[error("a JSON value could not be encoded")]
    Encode(#[source] serde_json::Error),
    /// The assembled server could not start, serve or stop.
    #[cfg(unix)]
    #[error(transparent)]
    Assembly(crate::assembly::AssemblyError),
    /// The attach ended without a detach.
    #[cfg(unix)]
    #[error(transparent)]
    Attach(crate::attach::AttachError),
}

impl CliError {
    /// The stable machine-readable kind.
    #[must_use]
    pub(super) const fn kind(&self) -> &'static str {
        match self {
            Self::Usage { .. } | Self::MissingCommand | Self::SkillWithCommand => "usage",
            Self::JsonSwitch { .. } => "invalid_json_switch",
            Self::Params(_) | Self::ParamsJson(_) => "invalid_params",
            Self::Command(error) => error.kind(),
            Self::UnknownMethod { .. } => "unknown_method",
            Self::NeedsTarget { .. } => "needs_target",
            Self::Endpoint(error) => error.kind(),
            Self::NotRunning { .. } => "server_not_running",
            Self::Io { .. } => "transport_failed",
            #[cfg(windows)]
            Self::PipeBusy { .. } => "pipe_busy_timeout",
            Self::Closed => "connection_closed",
            Self::Restarted => "server_restart",
            Self::Reply(_) | Self::Result(_) => "invalid_response",
            Self::Timeout { .. } => "deadline_exceeded",
            Self::UnsupportedTransport => "unsupported_transport",
            Self::Schema(_) | Self::Encode(_) => "internal",
            Self::ReleasedFlag => "usage",
            Self::BaselineUnreadable { .. } => "baseline_unreadable",
            Self::BaselineJson { .. } | Self::BaselineSchema { .. } => "baseline_malformed",
            Self::GenerationMismatch { .. } => "generation_mismatch",
            Self::CompatibilityBreak { .. } => "compatibility_break",
            #[cfg(any(unix, windows))]
            Self::Service(error) => error.kind(),
            #[cfg(unix)]
            Self::Assembly(error) => error.kind(),
            #[cfg(unix)]
            Self::Attach(error) => error.kind(),
        }
    }

    /// Recovery guidance for the kind.
    #[must_use]
    pub(super) const fn hint(&self) -> &'static str {
        match self {
            Self::Usage { .. } | Self::MissingCommand | Self::SkillWithCommand => {
                "run `ovis-terminal --json schema cli` for every command and flag"
            }
            Self::JsonSwitch { .. } => "set OVIS_JSON=1 for JSON, 0 or unset for text",
            Self::Params(_) | Self::ParamsJson(_) => {
                "correct the field at field_path; unknown fields are never ignored"
            }
            Self::Command(error) => error.hint(),
            Self::UnknownMethod { .. } => "run `ovis-terminal schema` for the method catalog",
            Self::NeedsTarget { .. } => {
                "repeat with --terminal-id, --address or --name naming one candidate"
            }
            Self::Endpoint(error) => error.hint(),
            Self::NotRunning { .. } => "start the server explicitly; inspection never starts one",
            Self::Io { .. } => "check the endpoint path and its permissions",
            #[cfg(windows)]
            Self::PipeBusy { .. } => {
                "every pipe instance stayed taken; retry, and check the server's accept loop keeps serving"
            }
            Self::Closed => "check the server log; the connection ended early",
            Self::Restarted => {
                "reconnect to the same endpoint; ask op.get whether a mutation committed"
            }
            Self::Reply(_) | Self::Result(_) => {
                "check that the endpoint belongs to an ovis-terminal server of this generation"
            }
            Self::Timeout { .. } => "inspect the operation; it may still be running",
            Self::UnsupportedTransport => "use a Unix build or the protocol directly",
            Self::Schema(_) | Self::Encode(_) => "report the build; a value could not be encoded",
            Self::ReleasedFlag => "run `ovis-terminal schema compat --released <path>`",
            Self::BaselineUnreadable { .. } => {
                "pass the path of a readable released OpenRPC document of at most 64 MiB"
            }
            Self::BaselineJson { .. } | Self::BaselineSchema { .. } => {
                "pass the released OpenRPC document (schema/released.openrpc.json), not another file"
            }
            Self::GenerationMismatch { .. } => {
                "compare with the released baseline of this build's generation"
            }
            Self::CompatibilityBreak { .. } => {
                "keep released methods and required outputs; add inputs only as optional"
            }
            #[cfg(any(unix, windows))]
            Self::Service(error) => error.hint(),
            #[cfg(unix)]
            Self::Assembly(error) => error.hint(),
            #[cfg(unix)]
            Self::Attach(error) => error.hint(),
        }
    }
}
