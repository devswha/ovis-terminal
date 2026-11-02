//! Argument shapes the typed method verbs share.
//!
//! One verb's flags and `--params` become the params object of its method. Planning reads
//! only the argument list and the invocation's own process token; nothing here opens a
//! connection.

use clap::Args;
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::{ParamsError, decode_params};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use thiserror::Error;

/// Proves a request's params decode as its method's closed params type.
pub(crate) type Check = fn(&RpcRequest) -> Result<(), ParamsError>;

/// [`Check`] for the closed params type `P`.
pub(crate) fn check<P: DeserializeOwned>(request: &RpcRequest) -> Result<(), ParamsError> {
    decode_params::<P>(request).map(drop)
}

/// Whether a verb's params name a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetUse {
    /// `target` is required; without a target flag the verb answers `needs_target`.
    Required,
    /// `target` narrows the result and may be left out.
    Optional,
    /// The params have no `target`.
    Absent,
}

/// One typed verb: the method it sends and what the params type requires.
#[derive(Debug, Clone, Copy)]
pub(crate) struct VerbSpec {
    /// Published method name.
    pub(crate) method: &'static str,
    /// Decodes the merged params as the method's closed params type.
    pub(crate) check: Check,
    /// The params carry the reporting program's `process_token`.
    pub(crate) token: bool,
    /// How the params name a terminal.
    pub(crate) target: TargetUse,
}

/// The typed `server` mutations and metrics read: metrics, stop, restart.
pub(crate) const SERVER: [VerbSpec; 3] = [
    VerbSpec {
        method: "server.metrics",
        check: check::<ovis_terminal_protocol::schema::common::NoParams>,
        token: false,
        target: TargetUse::Absent,
    },
    VerbSpec {
        method: "server.stop",
        check: check::<ovis_terminal_protocol::schema::methods::server::ServerStopParams>,
        token: false,
        target: TargetUse::Absent,
    },
    VerbSpec {
        method: "server.restart",
        check: check::<ovis_terminal_protocol::schema::methods::server::ServerStartParams>,
        token: false,
        target: TargetUse::Absent,
    },
];

/// At most one explicit way to name a terminal; there is no focus fallback.
#[derive(Debug, Clone, Default, Args)]
#[group(multiple = false)]
pub(crate) struct TargetFlags {
    /// Stable terminal identity.
    #[arg(long)]
    pub(crate) terminal_id: Option<String>,
    /// Terminal address such as `t7` or `machine/t7`.
    #[arg(long)]
    pub(crate) address: Option<String>,
    /// Unique live terminal name.
    #[arg(long)]
    pub(crate) name: Option<String>,
}

impl TargetFlags {
    fn target(self) -> Option<Value> {
        [
            ("terminal_id", self.terminal_id),
            ("address", self.address),
            ("name", self.name),
        ]
        .into_iter()
        .find_map(|(field, value)| value.map(|value| json!({ field: value })))
    }
}

/// The remaining params of the method as one JSON object.
#[derive(Debug, Clone, Args)]
pub(crate) struct ParamsArg {
    /// Params object as JSON text; flags fill `target` and `op_id`.
    #[arg(long, default_value = "{}")]
    pub(crate) params: String,
}

/// A read of one terminal: `[--terminal-id ID | --address ADDR | --name NAME] [--params JSON]`.
#[derive(Debug, Clone, Args)]
pub(crate) struct Read {
    #[command(flatten)]
    pub(crate) target: TargetFlags,
    #[command(flatten)]
    pub(crate) params: ParamsArg,
}

/// A read that names no terminal: `[--params JSON]`.
#[derive(Debug, Clone, Args)]
pub(crate) struct Plain {
    #[command(flatten)]
    pub(crate) params: ParamsArg,
}

/// A mutation of one terminal: `--op-id ID [target] [--params JSON]`.
#[derive(Debug, Clone, Args)]
pub(crate) struct Mutate {
    /// Client-minted operation identity; resending it never repeats the effect.
    #[arg(long)]
    pub(crate) op_id: String,
    #[command(flatten)]
    pub(crate) target: TargetFlags,
    #[command(flatten)]
    pub(crate) params: ParamsArg,
}

/// A mutation that names no terminal: `--op-id ID [--params JSON]`.
#[derive(Debug, Clone, Args)]
pub(crate) struct MutatePlain {
    /// Client-minted operation identity; resending it never repeats the effect.
    #[arg(long)]
    pub(crate) op_id: String,
    #[command(flatten)]
    pub(crate) params: ParamsArg,
}

/// What a verb invocation supplied, in one shape.
#[derive(Debug, Clone, Default)]
pub(crate) struct Supplied {
    target: Option<Value>,
    op_id: Option<String>,
    params: String,
}

impl From<Read> for Supplied {
    fn from(args: Read) -> Self {
        Self {
            target: args.target.target(),
            op_id: None,
            params: args.params.params,
        }
    }
}

impl From<Plain> for Supplied {
    fn from(args: Plain) -> Self {
        Self {
            target: None,
            op_id: None,
            params: args.params.params,
        }
    }
}

impl From<Mutate> for Supplied {
    fn from(args: Mutate) -> Self {
        Self {
            target: args.target.target(),
            op_id: Some(args.op_id),
            params: args.params.params,
        }
    }
}

impl From<MutatePlain> for Supplied {
    fn from(args: MutatePlain) -> Self {
        Self {
            target: None,
            op_id: Some(args.op_id),
            params: args.params.params,
        }
    }
}

/// A planned verb: the params to send, or the refusal to guess a target.
#[derive(Debug)]
pub(crate) enum Planned {
    /// Send `method` with `params` once they pass `check`.
    Send {
        /// The verb's method and params type.
        spec: VerbSpec,
        /// The merged params object.
        params: Value,
    },
    /// The method needs a terminal and none was named. `probe` is the merged params with a
    /// stand-in target: it must pass `check` before candidates are listed, so a malformed
    /// targetless invocation is refused as malformed. It is never sent.
    NeedsTarget {
        /// The verb's method and params type.
        spec: VerbSpec,
        /// The merged params with the stand-in address as the target.
        probe: Value,
    },
}

/// A terminal address every closed `target` accepts, standing in for the missing target
/// while the supplied fields are checked.
const STAND_IN_ADDRESS: &str = "t1";

/// Why a verb's flags and params could not be merged; nothing was sent.
#[derive(Debug, Error)]
pub(crate) enum CommandError {
    /// `--params` is not a JSON object.
    #[error("--params is not a JSON object")]
    ParamsJson(#[source] serde_json::Error),
    /// A field was given both as a flag and inside `--params`.
    #[error("{field} is given both as a flag and inside --params")]
    Repeated {
        /// The params field named twice.
        field: &'static str,
    },
}

impl CommandError {
    /// The stable machine-readable kind.
    pub(crate) const fn kind(&self) -> &'static str {
        match self {
            Self::ParamsJson(_) => "invalid_params",
            Self::Repeated { .. } => "usage",
        }
    }

    /// Recovery guidance for the kind.
    pub(crate) const fn hint(&self) -> &'static str {
        match self {
            Self::ParamsJson(_) => "pass --params as one JSON object, for example '{}'",
            Self::Repeated { .. } => "name the field once: either the flag or the params key",
        }
    }
}

fn insert(
    params: &mut Map<String, Value>,
    field: &'static str,
    value: Value,
) -> Result<(), CommandError> {
    if params.contains_key(field) {
        return Err(CommandError::Repeated { field });
    }
    params.insert(field.to_owned(), value);
    Ok(())
}

/// Merges `supplied` into the params of `spec`. The invocation's process `token` fills
/// `process_token` only when the params type carries one and `--params` names none.
pub(crate) fn plan(
    spec: VerbSpec,
    supplied: Supplied,
    token: Option<&str>,
) -> Result<Planned, CommandError> {
    let mut params: Map<String, Value> =
        serde_json::from_str(&supplied.params).map_err(CommandError::ParamsJson)?;
    if let Some(target) = supplied.target {
        insert(&mut params, "target", target)?;
    }
    if let Some(op_id) = supplied.op_id {
        insert(&mut params, "op_id", Value::String(op_id))?;
    }
    if let (true, Some(token), false) = (spec.token, token, params.contains_key("process_token")) {
        params.insert("process_token".to_owned(), Value::String(token.to_owned()));
    }
    if spec.target == TargetUse::Required && !params.contains_key("target") {
        params.insert("target".to_owned(), json!({ "address": STAND_IN_ADDRESS }));
        return Ok(Planned::NeedsTarget {
            spec,
            probe: Value::Object(params),
        });
    }
    Ok(Planned::Send {
        spec,
        params: Value::Object(params),
    })
}

#[cfg(test)]
mod tests;
