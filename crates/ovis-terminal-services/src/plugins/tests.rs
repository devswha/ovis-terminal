use std::path::PathBuf;

use ovis_terminal_protocol::schema::ids::PluginRunId;
use ovis_terminal_protocol::schema::methods::plugin::PluginInvokeParams;
use proptest::prelude::*;
use serde_json::{Value, json};

use super::input::Definition;
use super::runs::Tail;
use super::{PluginError, Plugins};

mod runs;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn at(value: &Value, pointer: &str) -> Value {
    value.pointer(pointer).cloned().unwrap_or(Value::Null)
}

fn set(value: &mut Value, field: &str, replacement: Value) -> TestResult {
    value
        .as_object_mut()
        .ok_or("not an object")?
        .insert(field.to_owned(), replacement);
    Ok(())
}

#[derive(Debug)]
struct Root(PathBuf);

impl Root {
    fn new(name: &str) -> TestResult<Self> {
        let root = std::env::temp_dir().join(format!("ovt46-{}-{name}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        std::fs::create_dir_all(&root)?;
        Ok(Self(root))
    }

    fn plugin(&self, id: &str, manifest: &Value) -> TestResult<PathBuf> {
        let directory = self.0.join(id);
        std::fs::create_dir_all(&directory)?;
        std::fs::write(directory.join("plugin.json"), serde_json::to_vec(manifest)?)?;
        Ok(directory)
    }

    fn plugins(&self) -> Plugins {
        Plugins::new(self.0.clone())
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.0);
        assert!(removed.is_ok(), "cleanup: {removed:?}");
    }
}

fn manifest(enabled: bool, actions: &Value) -> Value {
    json!({"plugin_id": "fixture", "version": "1.2.3", "enabled": enabled, "actions": actions})
}

fn sh_action(id: &str, script: &str) -> Value {
    json!({"action_id": id, "argv": ["/bin/sh", script], "contexts": ["machine"]})
}

fn invoke(action: &str, input: &Value) -> TestResult<PluginInvokeParams> {
    Ok(serde_json::from_value(json!({
        "op_id": "0191f1a2-0000-7000-8000-000000000046",
        "plugin_id": "fixture",
        "action_id": action,
        "input": input,
        "target": null,
    }))?)
}

fn run_id() -> PluginRunId {
    PluginRunId::new("plugin-run:test".to_owned())
}

#[test]
fn the_catalog_lists_every_unavailable_action_with_its_reason() -> TestResult {
    let root = Root::new("reasons")?;
    let other = if cfg!(target_os = "linux") {
        "macos"
    } else {
        "linux"
    };
    root.plugin(
        "fixture",
        &manifest(true, &json!([
            sh_action("ok", "/dev/null"),
            {"action_id": "missing", "argv": ["/nonexistent/ovis-plugin"], "contexts": ["machine"]},
            {"action_id": "relative", "argv": ["bin/tool"], "contexts": ["machine"]},
            {"action_id": "spliced", "argv": ["/bin/sh", "-c", "echo {name}"], "contexts": ["machine"],
             "input_schema": {"type": "object", "properties": {"name": {"type": "string"}},
                              "required": ["name"], "additionalProperties": false}},
            {"action_id": "elsewhere", "argv": ["/bin/sh"], "contexts": ["machine"], "platforms": [other]},
        ])),
    )?;

    let listing = serde_json::to_value(root.plugins().catalog()?.listing())?;

    let reasons: Vec<Value> = (0..5)
        .map(|index| at(&listing, &format!("/plugins/0/actions/{index}/unavailable")))
        .collect();
    assert_eq!(
        reasons,
        [
            Value::Null,
            json!("executable_missing"),
            json!("definition_invalid"),
            json!("definition_invalid"),
            json!("platform_unsupported"),
        ]
    );
    assert_eq!(at(&listing, "/plugins/0/version"), json!("1.2.3"));
    assert_eq!(
        at(&listing, "/plugins/0/actions/0/contexts"),
        json!(["machine"])
    );
    Ok(())
}

#[test]
fn a_disabled_plugin_lists_and_refuses_with_its_reason() -> TestResult {
    let root = Root::new("disabled")?;
    root.plugin(
        "fixture",
        &manifest(false, &json!([sh_action("ok", "/dev/null")])),
    )?;
    let plugins = root.plugins();

    let listing = serde_json::to_value(plugins.catalog()?.listing())?;
    let refused = plugins.prepare(run_id(), &invoke("ok", &json!({}))?);

    assert_eq!(at(&listing, "/plugins/0/enabled"), json!(false));
    assert_eq!(
        at(&listing, "/plugins/0/actions/0/unavailable"),
        json!("plugin_disabled")
    );
    assert_eq!(
        refused.map(|_| ()).map_err(|error| error.kind()),
        Err("plugin_disabled")
    );
    Ok(())
}

#[test]
fn malformed_or_mismatched_manifests_are_typed_failures() -> TestResult {
    let unknown = Root::new("unknown-field")?;
    let mut extra = manifest(true, &json!([]));
    set(&mut extra, "shell", json!("bash -c"))?;
    unknown.plugin("fixture", &extra)?;
    let renamed = Root::new("renamed")?;
    renamed.plugin("other", &manifest(true, &json!([])))?;

    let kinds = [unknown.plugins(), renamed.plugins()]
        .map(|plugins| plugins.catalog().map(|_| ()).map_err(|error| error.kind()));

    assert_eq!(kinds, [Err("plugin_invalid"), Err("plugin_invalid")]);
    Ok(())
}

#[test]
fn a_missing_root_has_no_plugins_and_unknown_actions_are_not_found() -> TestResult {
    let plugins = Plugins::new(std::env::temp_dir().join("ovt46-absent-root"));

    assert_eq!(plugins.catalog()?.listing().plugins.len(), 0);
    assert!(matches!(
        plugins.prepare(run_id(), &invoke("ok", &json!({}))?),
        Err(PluginError::NotFound)
    ));
    Ok(())
}

#[test]
fn input_and_context_are_checked_before_anything_starts() -> TestResult {
    let root = Root::new("input")?;
    let mut action = sh_action("greet", "/dev/null");
    set(
        &mut action,
        "input_schema",
        json!({"type": "object", "additionalProperties": false,
        "properties": {"name": {"type": "string"}, "count": {"type": "integer"}},
        "required": ["name"]}),
    )?;
    root.plugin("fixture", &manifest(true, &json!([action])))?;
    let plugins = root.plugins();
    let field = |input: Value| -> TestResult<Option<String>> {
        Ok(match plugins.prepare(run_id(), &invoke("greet", &input)?) {
            Err(PluginError::InvalidInput { field }) => Some(field),
            _ => None,
        })
    };

    assert_eq!(
        field(json!({"name": "a", "extra": 1}))?.as_deref(),
        Some("input.extra")
    );
    assert_eq!(field(json!({"name": 3}))?.as_deref(), Some("input.name"));
    assert_eq!(field(json!({"count": 3}))?.as_deref(), Some("input.name"));
    assert_eq!(field(json!(["name"]))?.as_deref(), Some("input"));
    let mut targeted = invoke("greet", &json!({"name": "a"}))?;
    targeted.target = Some(serde_json::from_value(json!({"terminal_id": "t1"}))?);
    assert!(matches!(
        plugins.prepare(run_id(), &targeted),
        Err(PluginError::ContextUnsupported)
    ));
    Ok(())
}

#[test]
fn a_running_run_reports_the_terminals_linked_to_it() -> TestResult {
    let runs = super::runs::Runs::default();
    let run = runs.start(run_id())?;
    let terminal = ovis_terminal_protocol::schema::ids::TerminalId::new("t7".to_owned());

    runs.link_created(&run_id(), terminal.clone())?;

    assert_eq!(run.record()?.created_terminals, vec![terminal]);
    assert!(matches!(runs.start(run_id()), Err(PluginError::RunExists)));
    Ok(())
}

fn echo_definition() -> Option<Definition> {
    let schema = json!({"type": "object", "additionalProperties": false,
        "properties": {"text": {"type": "string"}}, "required": ["text"]});
    Definition::parse(
        &["/bin/echo".to_owned(), "--".to_owned(), "{text}".to_owned()],
        &schema,
    )
}

proptest! {
    #[test]
    fn any_text_input_is_exactly_one_argument(text in "[^\u{0}]{0,64}") {
        let definition = echo_definition().ok_or_else(|| TestCaseError::fail("definition"))?;

        let expanded = definition
            .expand(&json!({"text": text}))
            .map_err(|error| TestCaseError::fail(error.to_string()))?;

        prop_assert_eq!(expanded.args, vec!["--".to_owned(), text]);
    }

    #[test]
    fn the_tail_keeps_the_newest_bytes_and_their_offset(
        chunks in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..300), 0..40),
        cap in 1_usize..512,
    ) {
        let mut tail = Tail::new(cap);
        let mut all = Vec::new();
        for chunk in &chunks {
            tail.push(chunk);
            all.extend_from_slice(chunk);
        }

        let kept = all.len().min(cap);
        let floor = all.len().saturating_sub(kept);
        prop_assert_eq!(tail.retained(), all.get(floor..).unwrap_or_default().to_vec());
        prop_assert_eq!(tail.floor().get(), u64::try_from(floor).unwrap_or(u64::MAX));
    }
}
