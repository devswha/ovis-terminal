use std::path::{Path, PathBuf};

use ovis_terminal_protocol::schema::ids::TemplateName;
use ovis_terminal_protocol::schema::methods::terminal::TerminalRunParams;
use ovis_terminal_protocol::schema::records::AgentState;
use ovis_terminal_protocol::schema::scalar::DurationMs;
use proptest::prelude::*;
use serde_json::{Value, json};

use super::{LaunchPlan, PlannedMessage, TemplateCatalog, TemplateError, TemplateVersion};

struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("ovis-t35-templates-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&path).expect("create the catalog directory");
        Self(path)
    }

    fn put(&self, file: &str, definition: &Value) {
        std::fs::write(self.0.join(file), definition.to_string()).expect("write a definition");
    }

    fn catalog(&self) -> TemplateCatalog {
        TemplateCatalog::new(&self.0)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.0);
        assert!(removed.is_ok(), "remove the owned catalog: {removed:?}");
    }
}

fn build(version: &str, argv: &[&str]) -> Value {
    json!({
        "name": "build",
        "version": version,
        "description": format!("builder {version}"),
        "argv": argv,
        "env": {"ROLE": "build"},
        "readiness": {"by": "declared", "states": ["idle"]},
        "body": "You are the build agent."
    })
}

fn params(extra: &Value) -> TerminalRunParams {
    let mut base = json!({"op_id": "0191f1a2-0000-7000-8000-000000000035", "geometry": {"cols": 80, "rows": 24}});
    if let (Some(target), Some(extra)) = (base.as_object_mut(), extra.as_object()) {
        target.extend(extra.clone());
    }
    serde_json::from_value(base).expect("terminal.run params")
}

fn set(definition: &mut Value, key: &str, value: Value) {
    if let Some(object) = definition.as_object_mut() {
        object.insert(key.to_owned(), value);
    }
}

fn name(text: &str) -> TemplateName {
    TemplateName::new(text.to_owned())
}

#[test]
fn a_pinned_version_resolves_that_version_and_an_unpinned_one_the_latest() {
    let dir = Dir::new();
    dir.put("build-1.json", &build("1", &["/usr/bin/make", "all"]));
    dir.put("build-10.json", &build("1.10", &["/usr/bin/make", "ten"]));
    dir.put("build-2.json", &build("1.2", &["/usr/bin/make", "two"]));
    let catalog = dir.catalog();
    let pinned = catalog.get(&name("build"), Some("1")).expect("build@1");
    assert_eq!(pinned.record().argv, ["/usr/bin/make", "all"]);
    let latest = catalog.get(&name("build"), None).expect("latest build");
    assert_eq!(latest.version().as_str(), "1.10");
    let listed: Vec<String> = catalog
        .list()
        .expect("list")
        .iter()
        .map(|template| template.version().to_string())
        .collect();
    assert_eq!(listed, ["1", "1.2", "1.10"]);
}

#[test]
fn an_unknown_name_or_version_is_template_not_found() {
    let dir = Dir::new();
    dir.put("build.json", &build("1", &["/usr/bin/make"]));
    let catalog = dir.catalog();
    for (wanted, version) in [
        ("build", Some("missing")),
        ("build", Some("2")),
        ("deploy", None),
    ] {
        let error = catalog.get(&name(wanted), version).expect_err("absent");
        assert_eq!(error.kind(), "template_not_found");
        assert!(matches!(error, TemplateError::NotFound { .. }));
    }
    let empty = TemplateCatalog::new(dir.0.join("absent"));
    assert_eq!(empty.list().expect("a missing directory is empty").len(), 0);
}

fn invalid_kind(definition: &Value) -> &'static str {
    let dir = Dir::new();
    dir.put("t.json", definition);
    match dir.catalog().list() {
        Ok(listed) => panic!("accepted {definition}: {listed:?}"),
        Err(error) => {
            assert!(matches!(error, TemplateError::Invalid { .. }), "{error:?}");
            error.kind()
        }
    }
}

#[test]
fn quiet_readiness_bools_and_unsafe_states_are_template_invalid() {
    let mut quiet = build("1", &["/bin/agent"]);
    set(&mut quiet, "readiness", json!({"by": "quiet", "ms": "500"}));
    let mut legacy = build("1", &["/bin/agent"]);
    set(&mut legacy, "ready_when_quiet", json!(true));
    let mut blocked = build("1", &["/bin/agent"]);
    set(
        &mut blocked,
        "readiness",
        json!({"by": "declared", "states": ["blocked"]}),
    );
    let mut working = build("1", &["/bin/agent"]);
    set(
        &mut working,
        "readiness",
        json!({"by": "declared", "states": ["working"]}),
    );
    let mut version = build("1", &["/bin/agent"]);
    set(&mut version, "version", json!("latest"));
    let mut empty = build("1", &[]);
    set(&mut empty, "argv", json!([]));
    let mut hidden = build("1", &["/bin/agent"]);
    set(&mut hidden, "name", json!("../etc"));
    let mut env = build("1", &["/bin/agent"]);
    set(&mut env, "env", json!({"A=B": "x"}));
    for definition in [quiet, legacy, blocked, working, version, empty, hidden, env] {
        assert_eq!(invalid_kind(&definition), "template_invalid");
    }
}

#[test]
fn a_duplicate_name_and_version_names_the_second_file() {
    let dir = Dir::new();
    dir.put("a.json", &build("1", &["/bin/a"]));
    dir.put("b.json", &build("1", &["/bin/b"]));
    let Err(TemplateError::Invalid { file, .. }) = dir.catalog().list() else {
        panic!("a duplicate must be refused");
    };
    assert_eq!(
        file.file_name().and_then(|name| name.to_str()),
        Some("b.json")
    );
}

fn plan_for(dir: &Path, extra: &Value) -> LaunchPlan {
    let launch = params(extra);
    TemplateCatalog::new(dir)
        .get(&name("build"), Some("1"))
        .expect("build@1")
        .plan(&launch, Some("$(touch PWNED); `id` ${HOME}"))
}

#[test]
fn task_text_reaches_only_the_first_message_and_argv_stays_literal() {
    let dir = Dir::new();
    dir.put("build.json", &build("1", &["/usr/bin/make", "all"]));
    let plan = plan_for(
        &dir.0,
        &json!({"first_message": {"text": "go", "deadline_ms": "250"}, "env": {"ROLE": "override", "X": "1"}}),
    );
    assert_eq!(plan.argv, ["/usr/bin/make", "all"]);
    assert_eq!(plan.env.get("ROLE").map(String::as_str), Some("override"));
    assert_eq!(plan.env.get("X").map(String::as_str), Some("1"));
    assert_eq!(plan.readiness, [AgentState::Idle]);
    let message = plan.first_message.expect("a first message");
    assert_eq!(
        message,
        PlannedMessage {
            text: "You are the build agent.\n\n$(touch PWNED); `id` ${HOME}\n\ngo".to_owned(),
            deadline: DurationMs::new(250),
        }
    );
    assert!(!dir.0.join("PWNED").exists());
}

#[test]
fn a_direct_program_uses_the_same_plan_with_idle_readiness_and_the_default_deadline() {
    let launch =
        params(&json!({"program": {"argv": ["/bin/cat"]}, "first_message": {"text": "hello"}}));
    let plan = LaunchPlan::direct(&["/bin/cat".to_owned()], &launch);
    assert_eq!(plan.argv, ["/bin/cat"]);
    assert_eq!(plan.readiness, [AgentState::Idle]);
    assert_eq!(
        plan.first_message
            .map(|message| (message.text, message.deadline)),
        Some(("hello".to_owned(), DurationMs::new(30_000)))
    );
    let silent = LaunchPlan::direct(&["/bin/cat".to_owned()], &params(&json!({})));
    assert_eq!(silent.first_message, None);
}

proptest! {
    #[test]
    fn versions_order_by_numeric_components(a in proptest::collection::vec(0u64..1000, 1..4), b in proptest::collection::vec(0u64..1000, 1..4)) {
        let text = |parts: &[u64]| parts.iter().map(u64::to_string).collect::<Vec<_>>().join(".");
        let left = TemplateVersion::parse(&text(&a)).expect("valid");
        let right = TemplateVersion::parse(&text(&b)).expect("valid");
        prop_assert_eq!(left.cmp(&right), a.cmp(&b));
        prop_assert_eq!(left.as_str(), text(&a));
    }
}
