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
    /// The server closed the connection before answering.
    #[error("the server closed the connection before answering")]
    Closed,
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
    /// A request or result could not be encoded.
    #[error("a JSON value could not be encoded")]
    Encode(#[source] serde_json::Error),
}

impl CliError {
    /// The stable machine-readable kind.
    #[must_use]
    pub(super) const fn kind(&self) -> &'static str {
        match self {
            Self::Usage { .. } | Self::MissingCommand | Self::SkillWithCommand => "usage",
            Self::JsonSwitch { .. } => "invalid_json_switch",
            Self::Params(_) | Self::ParamsJson(_) => "invalid_params",
            Self::UnknownMethod { .. } => "unknown_method",
            Self::NeedsTarget { .. } => "needs_target",
            Self::Endpoint(error) => error.kind(),
            Self::NotRunning { .. } => "server_not_running",
            Self::Io { .. } => "transport_failed",
            Self::Closed => "connection_closed",
            Self::Reply(_) | Self::Result(_) => "invalid_response",
            Self::Timeout { .. } => "deadline_exceeded",
            Self::UnsupportedTransport => "unsupported_transport",
            Self::Schema(_) | Self::Encode(_) => "internal",
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
            Self::UnknownMethod { .. } => "run `ovis-terminal schema` for the method catalog",
            Self::NeedsTarget { .. } => {
                "repeat with --terminal-id, --address or --name naming one candidate"
            }
            Self::Endpoint(error) => error.hint(),
            Self::NotRunning { .. } => "start the server explicitly; inspection never starts one",
            Self::Io { .. } => "check the endpoint path and its permissions",
            Self::Closed => "check the server log; the connection ended early",
            Self::Reply(_) | Self::Result(_) => {
                "check that the endpoint belongs to an ovis-terminal server of this generation"
            }
            Self::Timeout { .. } => "inspect the operation; it may still be running",
            Self::UnsupportedTransport => "use a Unix build or the protocol directly",
            Self::Schema(_) | Self::Encode(_) => "report the build; a value could not be encoded",
        }
    }
}
