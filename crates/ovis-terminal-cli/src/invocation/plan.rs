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
    Schema(SchemaPart),
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
        crate::commands::Planned::NeedsTarget => Ok(Plan::NeedsTarget),
        crate::commands::Planned::Send { spec, params } => {
            let request = envelope(spec.method, &params)?;
            (spec.check)(&request).map_err(CliError::Params)?;
            Ok(Plan::Call(request))
        }
    }
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
        Command::Schema { part } => Plan::Schema(part),
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
mod tests {
    use clap::Parser as _;

    use super::{Plan, plan};
    use crate::invocation::args::Cli;
    use crate::invocation::error::CliError;

    fn planned(args: &[&str]) -> Result<Plan, CliError> {
        let cli = Cli::try_parse_from(std::iter::once("ovis-terminal").chain(args.iter().copied()))
            .map_err(|source| CliError::Usage {
                detail: String::new(),
                source,
            })?;
        plan(cli, Some("inherited-token"))
    }

    #[test]
    fn an_explicit_target_becomes_the_typed_get_request() {
        let Ok(Plan::Call(request)) = planned(&["terminal", "get", "--name", "build"]) else {
            panic!("expected a call plan");
        };
        assert_eq!(request.method(), "terminal.get");
        assert_eq!(
            request.params(),
            &serde_json::json!({"target": {"name": "build"}})
        );
    }

    #[test]
    fn a_missing_target_plans_needs_target_and_two_targets_do_not_parse() {
        assert!(matches!(
            planned(&["terminal", "get"]),
            Ok(Plan::NeedsTarget)
        ));
        let both = planned(&["terminal", "get", "--name", "a", "--address", "t1"]);
        assert_eq!(both.map(drop).map_err(|error| error.kind()), Err("usage"));
    }

    #[test]
    fn call_refuses_hello_unknown_methods_and_non_object_params() {
        let kind = |args: &[&str]| planned(args).map(drop).map_err(|error| error.kind());
        assert_eq!(kind(&["call", "hello"]), Err("unknown_method"));
        assert_eq!(kind(&["call", "terminal.lst"]), Err("unknown_method"));
        assert_eq!(
            kind(&["call", "terminal.list", "--params", "[1]"]),
            Err("invalid_params")
        );
        assert!(kind(&["call", "terminal.list", "--params", "{\"page\":null}"]).is_ok());
    }

    #[test]
    fn a_numeric_op_id_is_refused_with_its_field_path() {
        let typed = super::typed::<ovis_terminal_protocol::schema::methods::server::OpRefParams>(
            "op.get",
            &serde_json::json!({"op_id": 7}),
        );
        let Err(CliError::Params(error)) = typed else {
            panic!("expected a params refusal");
        };
        assert_eq!(error.field_path(), "params.op_id");
    }
}
