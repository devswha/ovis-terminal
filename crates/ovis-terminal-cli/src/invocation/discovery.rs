//! Offline discovery: the manual, both contracts and the resolved endpoint. Nothing here
//! opens a socket, creates a file or starts a process.

use clap::CommandFactory as _;
use ovis_terminal_protocol::schema::{PROTOCOL_GENERATION, render_openrpc};
use ovis_terminal_server::listen::{EndpointDescriptor, EnvVar, Platform};
use serde_json::{Value, json};

use super::args::Cli;
use super::context::{CREDENTIAL_ENV, Context, JSON_ENV, OutputMode, PROCESS_TOKEN_ENV};
use super::error::CliError;
use super::exit::ExitStatus;
use super::report::Report;
use crate::skill::MANUAL;

/// The manual of this build, stamped with the version and generation it describes.
pub(super) fn skill(mode: OutputMode) -> Report {
    let generation = PROTOCOL_GENERATION.get();
    match mode {
        OutputMode::Json => Report::success(
            mode,
            &json!({ "version": crate::VERSION, "generation": generation.to_string(), "manual": MANUAL }),
        ),
        OutputMode::Text => Report::raw(format!(
            "<!-- {} {} protocol generation {generation} -->\n{MANUAL}",
            crate::BINARY_NAME,
            crate::VERSION
        )),
    }
}

/// The runtime contract reference of this build, generated from its types.
pub(super) fn contract(mode: OutputMode) -> Result<Report, CliError> {
    let reference = crate::skill::contract(&cli_contract()).map_err(CliError::Schema)?;
    Ok(match mode {
        OutputMode::Json => Report::success(
            mode,
            &json!({
                "version": crate::VERSION,
                "generation": PROTOCOL_GENERATION.get().to_string(),
                "contract": reference,
            }),
        ),
        OutputMode::Text => Report::raw(reference),
    })
}

/// The OpenRPC document this build serves, byte for byte as published.
pub(super) fn openrpc() -> Result<Report, CliError> {
    render_openrpc().map(Report::raw).map_err(CliError::Schema)
}

fn arguments(command: &clap::Command) -> Vec<Value> {
    command
        .get_arguments()
        .filter(|arg| !matches!(arg.get_id().as_str(), "help" | "version"))
        .map(|arg| {
            json!({
                "name": arg.get_id().as_str(),
                "long": arg.get_long(),
                "positional": arg.is_positional(),
                "required": arg.is_required_set(),
                "global": arg.is_global_set(),
            })
        })
        .collect()
}

fn commands(command: &clap::Command, prefix: &str, out: &mut Vec<Value>) {
    for sub in command.get_subcommands() {
        let path = format!("{prefix}{}", sub.get_name())
            .trim_start()
            .to_owned();
        if sub.has_subcommands() {
            commands(sub, &format!("{path} "), out);
        } else {
            out.push(json!({
                "command": path,
                "about": sub.get_about().map(ToString::to_string),
                "method": crate::agent::method_of(&path),
                "arguments": arguments(sub),
            }));
        }
    }
}

/// Kinds this binary reports itself that no exit code names, so they exit 1 (`failure`).
pub(super) const FAILURE_KINDS: [&str; 17] = [
    "transport_failed",
    "connection_closed",
    "invalid_response",
    "internal",
    "pipe_busy_timeout",
    "serve_failed",
    "stop_failed",
    "server_refused",
    "terminal_hung_up",
    "finished",
    "endpoint_insecure",
    "endpoint_io",
    "not_a_socket",
    "baseline_unreadable",
    "baseline_malformed",
    "generation_mismatch",
    "compatibility_break",
];

/// Commands, flags, environment and exit codes of this binary.
pub(super) fn cli_contract() -> Value {
    let root = Cli::command();
    let mut listed = Vec::new();
    commands(&root, "", &mut listed);
    let statuses: Vec<Value> = ExitStatus::ALL
        .into_iter()
        .map(|status| json!({ "code": status.code(), "name": status.name(), "kinds": status.kinds() }))
        .collect();
    let endpoint_vars = [
        EnvVar::SocketOverride,
        EnvVar::Session,
        EnvVar::RuntimeDir,
        EnvVar::StateDir,
        EnvVar::Home,
        EnvVar::User,
    ];
    let mut environment: Vec<&str> = endpoint_vars.into_iter().map(EnvVar::name).collect();
    environment.extend([JSON_ENV, CREDENTIAL_ENV, PROCESS_TOKEN_ENV]);
    json!({
        "binary": crate::BINARY_NAME,
        "version": crate::VERSION,
        "generation": PROTOCOL_GENERATION.get().to_string(),
        "flags": arguments(&root),
        "commands": listed,
        "exit_codes": statuses,
        "failure_kinds": FAILURE_KINDS,
        "environment": environment,
    })
}

/// The endpoint this invocation resolves to; the path is never inspected.
pub(super) fn endpoint(context: &Context) -> Result<Value, CliError> {
    let descriptor = EndpointDescriptor::resolve(context.endpoint(), Platform::CURRENT)
        .map_err(CliError::Endpoint)?;
    serde_json::to_value(descriptor).map_err(CliError::Encode)
}

/// A fresh operation identity: a `UUIDv7` minted now, so the server's retention window
/// admits it. Text prints the bare id for `--op-id "$(ovis-terminal op new)"`.
pub(super) fn new_op(mode: OutputMode) -> Report {
    let op_id = uuid::Uuid::now_v7().to_string();
    match mode {
        OutputMode::Json => Report::success(mode, &json!({ "op_id": op_id })),
        OutputMode::Text => Report::raw(format!("{op_id}\n")),
    }
}

/// `--version` in the requested form.
pub(super) fn version(mode: OutputMode) -> Report {
    match mode {
        OutputMode::Json => Report::success(
            mode,
            &json!({
                "binary": crate::BINARY_NAME,
                "version": crate::VERSION,
                "generation": PROTOCOL_GENERATION.get().to_string(),
            }),
        ),
        OutputMode::Text => Report::raw(format!("{} {}\n", crate::BINARY_NAME, crate::VERSION)),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{MANUAL, cli_contract};
    use crate::invocation::exit::ExitStatus;

    fn command_paths() -> Vec<String> {
        cli_contract()
            .get("commands")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                entry
                    .get("command")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect()
    }

    #[test]
    fn the_manual_names_every_command_and_every_exit_status() {
        for path in command_paths() {
            assert!(
                MANUAL.contains(&format!("`ovis-terminal {path}")),
                "manual omits `{path}`"
            );
        }
        for status in ExitStatus::ALL {
            assert!(
                MANUAL.contains(&format!("| {} | {} |", status.code(), status.name())),
                "manual omits exit status {}",
                status.name()
            );
        }
    }

    #[test]
    fn the_cli_contract_lists_the_grammar_and_every_exit_code() {
        let contract = cli_contract();
        let listed: Vec<&str> = contract
            .get("commands")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.get("command").and_then(Value::as_str))
            .collect();
        assert!(listed.contains(&"server status"));
        assert!(listed.contains(&"terminal get"));
        let codes: Vec<u64> = contract
            .get("exit_codes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.get("code").and_then(Value::as_u64))
            .collect();
        assert_eq!(codes, [0, 1, 2, 3, 4, 7, 9, 10, 11, 12, 13]);
    }
}
