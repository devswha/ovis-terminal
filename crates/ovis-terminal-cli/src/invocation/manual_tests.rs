//! The shipped examples and the typed verbs against this build's grammar.

use clap::{CommandFactory as _, Parser as _};
use serde_json::Value;

use super::args::Cli;
use super::discovery::cli_contract;
use super::exit::ExitStatus;
use crate::agent::VERBS;
use crate::skill;
use crate::skill::MANUAL;

const ONBOARDING: &str = include_str!("../../../../docs/producer-onboarding.md");

/// Parses and plans `args` (without the program name) as an invocation inside a spawned
/// program would, with an inherited process token, touching nothing: whether the plan is
/// `needs_target`, or the refusal's kind.
fn planned(args: &[&str]) -> Result<bool, &'static str> {
    let cli = Cli::try_parse_from(std::iter::once(crate::BINARY_NAME).chain(args.iter().copied()))
        .map_err(|_| "usage")?;
    super::plan::plan(cli, Some("inherited-token"))
        .map(|plan| matches!(plan, super::plan::Plan::NeedsTarget))
        .map_err(|error| error.kind())
}

/// The body lines of every fenced block whose info string is `info`.
pub(super) fn fenced<'a>(document: &'a str, info: &str) -> Vec<Vec<&'a str>> {
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

/// Splits one example command line: whitespace separates words, single quotes group.
fn words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut started = false;
    for character in line.chars() {
        match (character, quoted) {
            ('\'', _) => {
                quoted = !quoted;
                started = true;
            }
            (space, false) if space.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                }
                started = false;
            }
            (other, _) => {
                word.push(other);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

fn commands(document: &str) -> Vec<Vec<String>> {
    fenced(document, "sh")
        .into_iter()
        .flatten()
        .filter_map(|line| line.strip_prefix("ovis-terminal "))
        .map(words)
        .collect()
}

#[test]
fn every_example_command_parses_and_plans_against_this_build() {
    let examples: Vec<Vec<String>> = commands(MANUAL)
        .into_iter()
        .chain(commands(ONBOARDING))
        .collect();
    assert!(examples.len() >= 10, "{} examples", examples.len());
    for example in examples {
        let args: Vec<&str> = example.iter().map(String::as_str).collect();
        assert_eq!(planned(&args), Ok(false), "{args:?}");
    }
}

#[test]
fn a_targetless_verb_needs_a_target_only_when_every_supplied_field_is_valid() {
    assert_eq!(planned(&["agent", "get"]), Ok(true));
    let wait = ["turn", "wait", "--params", r#"{"timeout_ms":"1000"}"#];
    assert_eq!(planned(&wait), Ok(true));
    let unknown = ["agent", "get", "--params", r#"{"bogus":1}"#];
    assert_eq!(planned(&unknown), Err("invalid_params"));
    let numeric = [
        "status",
        "report",
        "--op-id",
        "x",
        "--params",
        r#"{"seq":1,"record":{"state":"idle"}}"#,
    ];
    assert_eq!(planned(&numeric), Err("invalid_params"));
    assert_eq!(
        planned(&["turn", "wait"]),
        Err("invalid_params"),
        "timeout_ms is required"
    );
}

#[test]
fn the_example_words_keep_quoted_json_whole() {
    assert_eq!(
        words("status report --params '{\"seq\":\"1\", \"x\":\"a b\"}' --address t1"),
        [
            "status",
            "report",
            "--params",
            "{\"seq\":\"1\", \"x\":\"a b\"}",
            "--address",
            "t1"
        ]
    );
}

#[test]
fn every_domain_subcommand_names_a_listed_verb() {
    let root = Cli::command();
    for group in ["agent", "status", "prompt", "turn", "template"] {
        let group = root
            .find_subcommand(group)
            .unwrap_or_else(|| panic!("{group} is not a command"));
        for sub in group.get_subcommands() {
            let method = format!("{}.{}", group.get_name(), sub.get_name().replace('-', "_"));
            assert!(
                VERBS.iter().any(|verb| verb.method == method),
                "{method} has no verb spec"
            );
        }
    }
    let counted: usize = ["agent", "status", "prompt", "turn", "template"]
        .iter()
        .filter_map(|group| root.find_subcommand(group))
        .map(|group| group.get_subcommands().count())
        .sum();
    assert_eq!(counted, VERBS.len());
}

#[test]
fn the_shipped_contract_reference_is_the_one_this_build_renders() {
    let rendered = skill::contract(&cli_contract()).expect("the contract renders");
    assert_eq!(
        include_str!("../../../../docs/runtime-contract.md"),
        rendered,
        "regenerate with `ovis-terminal schema contract > docs/runtime-contract.md`"
    );
}

#[test]
fn the_contract_reference_does_not_depend_on_the_platform_grammar() {
    let full = cli_contract();
    let mut without_commands = full.clone();
    if let Some(object) = without_commands.as_object_mut() {
        object.insert("commands".to_owned(), serde_json::json!([]));
    }
    assert_eq!(
        skill::contract(&full).expect("the contract renders"),
        skill::contract(&without_commands).expect("the contract renders"),
    );
}

#[test]
fn every_kind_this_binary_reports_is_published() {
    use super::error::CliError;

    let published: Vec<String> = ExitStatus::ALL
        .iter()
        .flat_map(|status| status.kinds().iter())
        .chain(super::discovery::FAILURE_KINDS.iter())
        .map(|kind| (*kind).to_owned())
        .collect();
    let json = serde_json::from_str::<Value>("[").expect_err("not JSON");
    let errors = [
        CliError::MissingCommand,
        CliError::SkillWithCommand,
        CliError::ParamsJson(json),
        CliError::UnknownMethod {
            method: String::new(),
        },
        CliError::NeedsTarget {
            candidates: Vec::new(),
            listed: false,
        },
        CliError::NotRunning {
            path: std::path::PathBuf::new(),
            state: "stopped",
        },
        CliError::Io {
            operation: "send",
            source: std::io::Error::other("x"),
        },
        CliError::Closed,
        CliError::Restarted,
        CliError::Timeout { deadline_ms: 1 },
        CliError::UnsupportedTransport,
        CliError::Command(crate::commands::CommandError::Repeated { field: "target" }),
    ];
    for error in errors {
        assert!(
            published.iter().any(|kind| kind == error.kind()),
            "{} is unpublished",
            error.kind()
        );
    }
}
