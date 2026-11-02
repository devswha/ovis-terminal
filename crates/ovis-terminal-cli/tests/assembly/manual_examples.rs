//! The documented surface is the served surface: every command line and params example the
//! manual and the producer onboarding guide show is executed against a running `serve`,
//! and hello advertises the method of every typed verb `schema cli` lists. A documented
//! command this build does not serve, or one that does not parse, fails here.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::Value;

use super::prompts::spawned_token;
use super::support::{BIN, Served, TestResult};

const MANUAL: &str = include_str!("../../../../docs/skill/ovis-terminal.md");
const ONBOARDING: &str = include_str!("../../../../docs/producer-onboarding.md");

/// The body lines of every fenced block whose info string is `info`.
fn fenced<'a>(document: &'a str, info: &str) -> Vec<Vec<&'a str>> {
    let mut blocks = Vec::new();
    let mut open: Option<Vec<&str>> = None;
    for line in document.lines() {
        match (&mut open, line.trim_start().strip_prefix("```")) {
            (None, Some(tag)) if tag.trim() == info => open = Some(Vec::new()),
            (Some(_), Some("")) => blocks.extend(open.take()),
            (Some(lines), _) => lines.push(line),
            (None, _) => {}
        }
    }
    blocks
}

/// Runs one documented line through `/bin/sh` as the program owning `t1` would: the binary
/// on `PATH`, the server's endpoint and bearer, and the program's own process token.
fn as_documented(served: &Served, token: &str, line: &str) -> TestResult<Output> {
    let dir = Path::new(BIN).parent().ok_or("binary directory")?;
    let socket = served
        .ready()
        .pointer("/ready/socket")
        .and_then(Value::as_str)
        .ok_or("socket")?;
    Ok(Command::new("/bin/sh")
        .args(["-c", line])
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
        .env("OVIS_TERMINAL_SOCKET", socket)
        .env("OVIS_TERMINAL_CREDENTIAL", served.bearer())
        .env("OVIS_TERMINAL_PROCESS_TOKEN", token)
        .stdin(Stdio::null())
        .output()?)
}

/// The error kind a failed run reported, or `None` on success.
fn refused(output: &Output) -> Option<String> {
    if output.status.success() {
        return None;
    }
    let record: Value = serde_json::from_slice(&output.stderr).unwrap_or(Value::Null);
    Some(
        record
            .pointer("/error/kind")
            .and_then(Value::as_str)
            .unwrap_or("unparsed")
            .to_owned(),
    )
}

/// Kinds that mean the documented example is not this build's surface: unserved,
/// unparsed or malformed. Any other outcome (success, `not_owner`, `not_found`, a
/// timeout) shows the method was reached with well-formed params.
const NOT_SERVED: [&str; 5] = [
    "method_not_found",
    "unknown_method",
    "usage",
    "invalid_params",
    "unparsed",
];

fn served_or_panic(what: &str, output: &Output) {
    let kind = refused(output);
    assert!(
        !kind
            .as_deref()
            .is_some_and(|kind| NOT_SERVED.contains(&kind)),
        "{what}: {kind:?}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn every_documented_command_and_params_example_is_served_by_this_build() -> TestResult {
    let served = Served::start()?;
    served.run_shell("shell")?;
    let token = spawned_token(&served)?;
    let mut executed = 0_usize;
    for document in [MANUAL, ONBOARDING] {
        for line in fenced(document, "sh").into_iter().flatten() {
            if line.trim_start().starts_with("ovis-terminal ") {
                served_or_panic(line, &as_documented(&served, &token, line)?);
                executed += 1;
            }
        }
        for block in fenced(document, "json") {
            let Some(method) = block
                .first()
                .and_then(|line| line.trim().strip_prefix("// params of "))
            else {
                continue;
            };
            let params = block.get(1..).unwrap_or_default().join("\n");
            let output = served.cli(&["call", method, "--params", &params])?;
            served_or_panic(method, &output);
            executed += 1;
        }
    }
    assert!(executed >= 20, "{executed} documented examples executed");
    served.stop()?;
    Ok(())
}

#[test]
fn hello_advertises_the_method_of_every_typed_verb() -> TestResult {
    let served = Served::start()?;
    let advertised = served.advertised()?;
    let contract = served.ok(&["schema", "cli"])?;
    let methods: Vec<&str> = contract
        .get("commands")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("method").and_then(Value::as_str))
        .collect();
    assert!(methods.len() >= 20, "{methods:?}");
    let unserved: Vec<&&str> = methods
        .iter()
        .filter(|method| !advertised.contains(**method))
        .collect();
    assert!(
        unserved.is_empty(),
        "typed verbs this build does not serve: {unserved:?}"
    );
    served.stop()?;
    Ok(())
}
