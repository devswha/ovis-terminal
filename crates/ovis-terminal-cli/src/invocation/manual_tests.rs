//! The documented commands and the typed verbs against this build's grammar.

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

/// Typed verb groups; a documented `<group> <verb>` must be a command of this build.
const GROUPS: [&str; 5] = ["agent", "status", "prompt", "turn", "template"];

/// Every `<group> <verb>` the text names in a code span or an `ovis-terminal` line.
fn named_verbs(document: &str) -> Vec<String> {
    let spans = document.split('`').skip(1).step_by(2);
    let lines = document
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("ovis-terminal "));
    spans
        .map(|span| span.strip_prefix("ovis-terminal ").unwrap_or(span))
        .chain(lines)
        .filter_map(|text| {
            let mut words = text.split_whitespace().filter(|word| *word != "--json");
            let (group, verb) = (words.next()?, words.next()?);
            let plain = verb.chars().all(|c| c.is_ascii_lowercase() || c == '-');
            (GROUPS.contains(&group) && plain).then(|| format!("{group} {verb}"))
        })
        .collect()
}

#[test]
fn every_typed_verb_the_docs_name_is_a_command_of_this_build() {
    let contract = cli_contract();
    let commands: Vec<&str> = contract
        .get("commands")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("command").and_then(Value::as_str))
        .collect();
    let named: Vec<String> = named_verbs(MANUAL)
        .into_iter()
        .chain(named_verbs(ONBOARDING))
        .collect();
    assert!(named.len() >= 20, "{} documented verbs", named.len());
    for verb in named {
        assert!(
            commands.contains(&verb.as_str()),
            "the docs name `{verb}`, which this build has no command for"
        );
    }
}

#[test]
fn every_typed_verb_command_names_the_method_it_sends() {
    let contract = cli_contract();
    let methods: Vec<&str> = contract
        .get("commands")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("method").and_then(Value::as_str))
        .collect();
    let typed: Vec<&str> = VERBS
        .into_iter()
        .chain(crate::commands::SERVER)
        .map(|verb| verb.method)
        .collect();
    assert_eq!(methods.len(), typed.len(), "{methods:?}");
    for method in typed {
        assert!(methods.contains(&method), "{method} has no command");
    }
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

#[test]
fn op_new_prints_a_fresh_v7_id_each_time() {
    let ids: Vec<uuid::Uuid> = (0..2)
        .map(|_| {
            let report = super::discovery::new_op(super::context::OutputMode::Text);
            let text = String::from_utf8_lossy(report.stdout()).into_owned();
            let id = text.strip_suffix('\n').expect("one line");
            uuid::Uuid::parse_str(id).expect("a UUID")
        })
        .collect();
    assert!(ids.iter().all(|id| id.get_version_num() == 7), "{ids:?}");
    assert_ne!(ids.first(), ids.last());
}
