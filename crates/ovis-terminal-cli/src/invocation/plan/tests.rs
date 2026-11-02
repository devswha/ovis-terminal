//! Planning refuses malformed invocations before any connection.

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
