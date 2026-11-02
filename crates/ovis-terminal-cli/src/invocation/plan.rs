//! Turning parsed arguments into typed requests. Planning touches no file or socket, so
//! every malformed invocation is refused before a connection is attempted.

use ovis_terminal_protocol::rpc::{RpcRequest, parse_request};
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_protocol::schema::methods::all_methods;
use ovis_terminal_protocol::schema::methods::server::OpRefParams;
use ovis_terminal_protocol::schema::methods::terminal::{TerminalGetParams, TerminalListParams};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use super::args::{
    CallArgs, Cli, Command, OpCommand, SchemaPart, ServerCommand, TargetArgs, TerminalCommand,
};
use super::error::CliError;

/// Request id of the one call an invocation sends.
pub(super) const REQUEST_ID: &str = "cli";

/// What an invocation does once its arguments are valid.
#[derive(Debug)]
pub(super) enum Plan {
    Skill,
    NewOp,
    Schema(SchemaPart),
    Compat(std::path::PathBuf),
    Endpoint,
    Bridge,
    Status,
    Call(RpcRequest),
    NeedsTarget,
    #[cfg(any(unix, windows))]
    Service(crate::service::ServiceCommand),
    #[cfg(unix)]
    Serve(crate::assembly::ServePlan),
    #[cfg(unix)]
    Attach(crate::attach::AttachPlan),
}

/// Builds a request and proves its params decode as the method's closed type `P`.
pub(super) fn typed<P: DeserializeOwned>(
    method: &str,
    params: &Value,
) -> Result<RpcRequest, CliError> {
    let request = envelope(method, params)?;
    decode_params::<P>(&request).map_err(CliError::Params)?;
    Ok(request)
}

fn envelope(method: &str, params: &Value) -> Result<RpcRequest, CliError> {
    let envelope =
        json!({ "jsonrpc": "2.0", "id": REQUEST_ID, "method": method, "params": params });
    parse_request(&envelope.to_string()).map_err(|_| CliError::UnknownMethod {
        method: method.to_owned(),
    })
}

fn target(args: TargetArgs) -> Option<Value> {
    let named = [
        ("terminal_id", args.terminal_id),
        ("address", args.address),
        ("name", args.name),
    ];
    named
        .into_iter()
        .find_map(|(field, value)| value.map(|value| json!({ "target": { field: value } })))
}

fn call(args: CallArgs) -> Result<RpcRequest, CliError> {
    let published = all_methods().any(|spec| spec.name == args.method);
    if !published || args.method == "hello" {
        return Err(CliError::UnknownMethod {
            method: args.method,
        });
    }
    let params: Map<String, Value> =
        serde_json::from_str(&args.params).map_err(CliError::ParamsJson)?;
    envelope(&args.method, &Value::Object(params))
}

/// Plans one typed verb: its flags merged into params that decode as its closed type.
fn verb(
    method: &'static str,
    supplied: crate::commands::Supplied,
    token: Option<&str>,
) -> Result<Plan, CliError> {
    let spec = crate::agent::VERBS
        .into_iter()
        .chain(crate::commands::SERVER)
        .find(|spec| spec.method == method)
        .ok_or_else(|| CliError::UnknownMethod {
            method: method.to_owned(),
        })?;
    match crate::commands::plan(spec, supplied, token).map_err(CliError::Command)? {
        crate::commands::Planned::NeedsTarget { spec, probe } => {
            checked(spec, &probe)?;
            Ok(Plan::NeedsTarget)
        }
        crate::commands::Planned::Send { spec, params } => Ok(Plan::Call(checked(spec, &params)?)),
    }
}

/// The verb's request for `params`, proven to decode as its closed params type.
fn checked(spec: crate::commands::VerbSpec, params: &Value) -> Result<RpcRequest, CliError> {
    let request = envelope(spec.method, params)?;
    (spec.check)(&request).map_err(CliError::Params)?;
    Ok(request)
}

/// Plans `cli`, or refuses it with nothing sent; `token` is the process token this
/// invocation inherited, which only a reporting verb sends.
pub(super) fn plan(cli: Cli, token: Option<&str>) -> Result<Plan, CliError> {
    let command = match (cli.skill, cli.command) {
        (true, None) => return Ok(Plan::Skill),
        (true, Some(_)) => return Err(CliError::SkillWithCommand),
        (false, None) => return Err(CliError::MissingCommand),
        (false, Some(command)) => command,
    };
    Ok(match command {
        Command::Connect => Plan::Bridge,
        Command::Schema { part, released } => super::compat::plan(part, released)?,
        Command::Endpoint => Plan::Endpoint,
        Command::Server(ServerCommand::Status) => Plan::Status,
        Command::Server(ServerCommand::Metrics(args)) => {
            verb("server.metrics", args.into(), token)?
        }
        Command::Server(ServerCommand::Stop(args)) => verb("server.stop", args.into(), token)?,
        Command::Server(ServerCommand::Restart(args)) => {
            verb("server.restart", args.into(), token)?
        }
        Command::Domain(command) => {
            let (method, supplied) = command.resolve();
            verb(method, supplied, token)?
        }
        Command::Terminal(TerminalCommand::List) => {
            Plan::Call(typed::<TerminalListParams>("terminal.list", &json!({}))?)
        }
        Command::Terminal(TerminalCommand::Get(args)) => match target(args) {
            Some(params) => Plan::Call(typed::<TerminalGetParams>("terminal.get", &params)?),
            None => Plan::NeedsTarget,
        },
        Command::Op(OpCommand::New) => Plan::NewOp,
        Command::Op(OpCommand::Get(args)) => Plan::Call(typed::<OpRefParams>(
            "op.get",
            &json!({ "op_id": args.op_id }),
        )?),
        Command::Op(OpCommand::Cancel(args)) => Plan::Call(typed::<OpRefParams>(
            "op.cancel",
            &json!({ "op_id": args.op_id }),
        )?),
        Command::Call(args) => Plan::Call(call(args)?),
        #[cfg(any(unix, windows))]
        Command::Service(command) => Plan::Service(command),
        #[cfg(unix)]
        Command::Serve(args) => Plan::Serve(crate::assembly::ServePlan {
            root: args.root,
            stdin_lifetime: args.stdin_lifetime,
            websocket_port: args.websocket_port,
            qa: crate::assembly::QaSeams {
                holder_poll_ms: args.qa_holder_poll_ms,
                actor_hold: args.qa_actor_hold,
                graphics_sequence_max_bytes: args.qa_graphics_sequence_max_bytes,
                output_ring_bytes: args.qa_output_ring_bytes,
            },
        }),
        #[cfg(unix)]
        Command::Hold(session) => Plan::Service(crate::service::ServiceCommand::Run(
            crate::service::unix::StartArgs {
                session,
                manager: crate::service::unix::Placement::Detached,
            },
        )),
        #[cfg(unix)]
        Command::Terminal(TerminalCommand::Attach(args)) => Plan::Attach(attach(args)?),
    })
}

/// Proves the target and client label decode as their closed wire types.
#[cfg(unix)]
fn attach(args: super::args::AttachArgs) -> Result<crate::attach::AttachPlan, CliError> {
    let super::args::AttachTarget {
        target,
        terminal_id,
        name,
    } = args.target;
    let named = [
        ("address", target),
        ("terminal_id", terminal_id),
        ("name", name),
    ];
    let target = named
        .into_iter()
        .find_map(|(field, value)| value.map(|value| json!({ field: value })))
        .ok_or(CliError::MissingCommand)?;
    let request = typed::<ovis_terminal_protocol::schema::methods::attach::AttachLeasesParams>(
        "attach.leases",
        &json!({ "target": target }),
    )?;
    let params: ovis_terminal_protocol::schema::methods::attach::AttachLeasesParams =
        decode_params(&request).map_err(CliError::Params)?;
    let label = args
        .client_id
        .unwrap_or_else(|| format!("cli-{}", std::process::id()));
    let client = serde_json::from_value(json!(label)).map_err(CliError::ParamsJson)?;
    let takeover = args
        .takeover
        .map(|epoch| serde_json::from_value(json!(epoch)))
        .transpose()
        .map_err(CliError::ParamsJson)?;
    Ok(crate::attach::AttachPlan {
        target: params.target,
        observe: args.observe,
        client,
        takeover,
    })
}

#[cfg(test)]
mod tests;
