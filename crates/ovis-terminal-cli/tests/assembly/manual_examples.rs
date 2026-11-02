//! The documented surface is the served surface: every command line and params example the
//! manual and the producer onboarding guide show is executed, in document order, against a
//! running `serve`, and must succeed unless the example declares the failure it shows
//! (`# exits <code> <kind>`). Hello advertises the method of every typed verb `schema cli`
//! lists.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::Value;

use super::prompts::spawned_token;
use super::support::{BIN, Served, TestResult, op_id};

const MANUAL: &str = include_str!("../../../../docs/skill/ovis-terminal.md");
const ONBOARDING: &str = include_str!("../../../../docs/producer-onboarding.md");
/// Placeholders a params example writes for the values a producer fills in.
const OP_PLACEHOLDER: &str = "<op id from ovis-terminal op new>";
const TOKEN_PLACEHOLDER: &str = "<OVIS_TERMINAL_PROCESS_TOKEN>";

/// One documented example, in document order.
enum Example {
    /// An `ovis-terminal ...` line of an `sh` block, run by `/bin/sh`.
    Line(String),
    /// A `// params of <method>` JSON block, sent with `call`.
    Params { method: String, body: String },
}

/// Every example of `document`, in the order a reader meets them.
fn examples(document: &str) -> Vec<Example> {
    let mut found = Vec::new();
    let mut open: Option<(&str, Vec<&str>)> = None;
    for line in document.lines() {
        match (&mut open, line.trim_start().strip_prefix("```")) {
            (None, Some(tag)) => open = Some((tag.trim(), Vec::new())),
            (Some((info, lines)), Some("")) => {
                found.extend(block(info, lines));
                open = None;
            }
            (Some((_, lines)), _) => lines.push(line),
            (None, None) => {}
        }
    }
    found
}

fn block(info: &str, lines: &[&str]) -> Vec<Example> {
    match (info, lines.split_first()) {
        ("sh", _) => lines
            .iter()
            .filter(|line| line.trim_start().starts_with("ovis-terminal "))
            .map(|line| Example::Line((*line).to_owned()))
            .collect(),
        ("json", Some((header, body))) => header
            .trim()
            .strip_prefix("// params of ")
            .map(|method| Example::Params {
                method: method.to_owned(),
                body: body.join("\n"),
            })
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

/// The exit code and kind a line declares with `# exits <code> <kind>`; none means 0.
fn declared(line: &str) -> Option<(i32, String)> {
    let marker = line.split_once("# exits ")?.1;
    let (code, kind) = marker.trim().split_once(' ')?;
    Some((code.parse().ok()?, kind.trim().to_owned()))
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

/// Requires the documented outcome: exit 0, or exactly the declared code and kind.
fn documented_outcome(what: &str, expected: Option<(i32, String)>, output: &Output) -> Value {
    let record: Value = serde_json::from_slice(&output.stderr).unwrap_or(Value::Null);
    let kind = record.pointer("/error/kind").and_then(Value::as_str);
    let (code, want) = expected.map_or((0, None), |(code, kind)| (code, Some(kind)));
    assert!(
        output.status.code() == Some(code) && kind == want.as_deref(),
        "{what}: expected exit {code} {want:?}, got {:?} {kind:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or(Value::Null)
}

/// Executes every example of `document` against a fresh `serve` whose `t1` runs the owning
/// shell; returns each example with its parsed stdout.
fn execute(document: &str) -> TestResult<Vec<(String, Value)>> {
    let served = Served::start()?;
    served.run_shell("shell")?;
    let token = spawned_token(&served)?;
    let mut results = Vec::new();
    for example in examples(document) {
        let (what, expected, output) = match example {
            Example::Line(line) => {
                let output = as_documented(&served, &token, &line)?;
                (line.clone(), declared(&line), output)
            }
            Example::Params { method, body } => {
                let params = body
                    .replace(OP_PLACEHOLDER, &op_id()?)
                    .replace(TOKEN_PLACEHOLDER, &token);
                let output = served.cli(&["call", &method, "--params", &params])?;
                (method, None, output)
            }
        };
        let stdout = documented_outcome(&what, expected, &output);
        results.push((what, stdout));
    }
    served.stop()?;
    Ok(results)
}

/// The parsed stdout of every executed example whose text contains `needle`, in order.
fn outputs<'a>(results: &'a [(String, Value)], needle: &str) -> Vec<&'a Value> {
    results
        .iter()
        .filter(|(what, _)| what.contains(needle))
        .map(|(_, stdout)| stdout)
        .collect()
}

#[test]
fn every_manual_example_succeeds_against_serve() -> TestResult {
    let results = execute(MANUAL)?;
    assert!(results.len() >= 5, "{} manual examples", results.len());
    Ok(())
}

#[test]
fn the_onboarding_flow_registers_reports_and_clears_with_the_owning_token() -> TestResult {
    let results = execute(ONBOARDING)?;
    assert!(results.len() >= 15, "{} onboarding examples", results.len());
    let agent = outputs(&results, "agent get --address t1");
    assert_eq!(
        agent
            .first()
            .and_then(|record| record.pointer("/agent/label")),
        Some(&Value::from("synthetic-producer")),
        "{agent:?}"
    );
    let status = outputs(&results, "status get --address t1");
    assert_eq!(
        status
            .first()
            .and_then(|record| record.pointer("/status/root/report/state")),
        Some(&Value::from("working")),
        "{status:?}"
    );
    assert_eq!(
        status
            .last()
            .and_then(|record| record.pointer("/status/root")),
        Some(&Value::Null),
        "the revocation step leaves no reported status: {status:?}"
    );
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
