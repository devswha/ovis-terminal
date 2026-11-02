use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{Value, json};

use super::contract::{self, Contract};
use super::guide::{self, Fate, Guide};
use super::{Injection, injected, policy};

fn row(path: &str, fate: &str, examples: &[&str]) -> String {
    let block = json!({"path": path, "fate": fate, "methods": ["prompt.answer"],
        "unserved": ["agent.note"], "events": ["prompt.opened"],
        "capabilities": ["capabilities.transports"], "identity": ["PromptId"],
        "fixtures": ["fixture:prompt-cards"], "examples": examples});
    format!("prose before\n```json ovis-migration\n{block}\n```\nprose after\n")
}

fn example(id: &str, steps: &Value) -> String {
    let case = json!({"format": "ovis-replay/1", "id": id, "server": "protocol", "kind": "happy",
        "ideals": [], "gaps": [], "excludes": [], "adversarial": [], "steps": steps});
    format!("```json ovis-example\n{case}\n```\n")
}

fn at(value: &Value, pointer: &str) -> Value {
    value.pointer(pointer).cloned().unwrap_or(Value::Null)
}

fn call(method: &str, params: &Value) -> Value {
    json!({"call": {"conn": "ovis", "id": method, "method": method, "params": params, "expect": {}}})
}

fn parsed(text: &str) -> Guide {
    Guide::parse((text, Path::new("guide.md"))).expect("the guide parses")
}

fn expected(paths: &[(&str, Fate)]) -> BTreeMap<String, Fate> {
    paths
        .iter()
        .map(|(path, fate)| ((*path).to_owned(), *fate))
        .collect()
}

#[test]
fn a_complete_guide_covers_every_path_with_its_fate_and_cites_every_example() {
    let text =
        row("a.ts", "replace", &["one"]) + &row("b.ts", "keep", &[]) + &example("one", &json!([]));
    let report = guide::coverage(
        &expected(&[("a.ts", Fate::Replace), ("b.ts", Fate::Keep)]),
        &parsed(&text),
    );
    assert_eq!(report, guide::complete());
}

#[test]
fn a_dropped_path_a_wrong_fate_and_a_dangling_example_are_each_reported() {
    let text = row("a.ts", "delete", &["missing"])
        + &row("a.ts", "replace", &[])
        + &example("loose", &json!([]));
    let report = guide::coverage(
        &expected(&[("a.ts", Fate::Replace), ("b.ts", Fate::Keep)]),
        &parsed(&text),
    );
    assert_eq!(at(&report, "/missing_paths"), json!(["b.ts"]));
    assert_eq!(at(&report, "/duplicate_paths"), json!(["a.ts"]));
    assert_eq!(
        at(&report, "/wrong_fate"),
        json!([{"path": "a.ts", "expected": "replace", "guide": "delete"}])
    );
    assert_eq!(at(&report, "/undefined_examples"), json!(["missing"]));
    assert_eq!(at(&report, "/uncited_examples"), json!(["loose"]));
}

#[test]
fn an_unclosed_block_and_an_unknown_row_field_are_refused() {
    let unclosed = Guide::parse(("```json ovis-migration\n{}\n", Path::new("g.md")));
    assert_eq!(
        unclosed.map(drop).map_err(|e| e.kind()),
        Err("scenario_inconsistent")
    );
    let typo = "```json ovis-migration\n{\"path\":\"a\",\"fate\":\"keep\",\"method\":[]}\n```\n";
    assert!(Guide::parse((typo, Path::new("g.md"))).is_err());
}

fn policy_of(steps: &Value) -> Value {
    policy::report(&parsed(&example("x", steps)).examples)
}

#[test]
fn keys_beside_a_prompt_a_resend_under_a_new_op_and_content_as_params_break_the_policy() {
    let keys = policy_of(&json!([
        call("prompt.get", &json!({})),
        call("input.keys", &json!({"keys": ["enter"]}))
    ]));
    assert_eq!(at(&keys, "/guessed_keys"), json!(["x"]));
    let send = json!({"target": {"name": "t"}, "text": "y", "submit": "enter"});
    let mut first = send.clone();
    first
        .as_object_mut()
        .expect("params")
        .insert("op_id".to_owned(), json!("$op:first"));
    let mut again = send;
    again
        .as_object_mut()
        .expect("params")
        .insert("op_id".to_owned(), json!("$op:again"));
    let resent = policy_of(&json!([
        call("input.send", &first),
        call("input.send", &again)
    ]));
    assert_eq!(at(&resent, "/auto_resend"), json!(["x"]));
    let idempotent = policy_of(&json!([
        call("input.send", &first),
        call("input.send", &first)
    ]));
    assert_eq!(
        at(&idempotent, "/auto_resend"),
        json!([]),
        "the same op id is the retained result"
    );
    let mut read = call("terminal.screen", &json!({}));
    *read.pointer_mut("/call/expect").expect("expect") =
        json!({"/result/lines/0": {"$capture": "line"}});
    let obeyed = policy_of(&json!([
        read,
        call("input.send", &json!({"text": "$value:line"}))
    ]));
    assert_eq!(at(&obeyed, "/content_as_command"), json!(["x"]));
}

#[test]
fn delivery_is_shown_only_by_an_op_get_of_the_sent_operation() {
    let send = call("input.send", &json!({"op_id": "$op:send"}));
    let asked = policy_of(&json!([
        send,
        call("op.get", &json!({"op_id": "$op:send"}))
    ]));
    assert_eq!(asked, policy::kept());
    let other = policy_of(&json!([
        send,
        call("op.get", &json!({"op_id": "$op:other"}))
    ]));
    assert_eq!(at(&other, "/unknown_delivery_shown"), json!(false));
}

#[test]
fn an_injection_rewrites_one_call_and_replaces_what_follows_it() {
    let text = example(
        "p",
        &json!([
            call("prompt.get", &json!({"prompt_id": "p1"})),
            call(
                "prompt.answer",
                &json!({"version": "$value:version", "prompt_id": "p1"})
            ),
            call("prompt.list", &json!({}))
        ]),
    );
    let guide = parsed(&text);
    let (_, case) = guide.examples.first().expect("one example");
    let injection: Injection = serde_json::from_value(json!({"request": {"example": "p"}, "example": "p",
        "step": "prompt.answer", "params": {"version": "0"},
        "expect": {"/error/data/kind": "version_conflict"}, "then": [call("prompt.get", &json!({}))]}))
    .expect("an injection");
    let rewritten = injected(case, &injection).expect("rewritten");
    let methods: Vec<Value> = (0..3)
        .map(|step| at(&rewritten, &format!("/steps/{step}/call/method")))
        .collect();
    assert_eq!(
        methods,
        [
            json!("prompt.get"),
            json!("prompt.answer"),
            json!("prompt.get")
        ]
    );
    assert_eq!(
        at(&rewritten, "/steps/3"),
        Value::Null,
        "the steps after the call are replaced"
    );
    assert_eq!(
        at(&rewritten, "/steps/1/call/params"),
        json!({"version": "0", "prompt_id": "p1"})
    );
    assert_eq!(
        at(&rewritten, "/steps/1/call/expect"),
        json!({"/error/data/kind": "version_conflict"})
    );
    let unknown = Injection {
        step: "nope".to_owned(),
        ..injection
    };
    assert_eq!(
        injected(case, &unknown).map(drop).map_err(|e| e.kind()),
        Err("scenario_inconsistent")
    );
}

#[test]
fn rows_may_list_only_served_methods_and_must_list_unserved_ones_as_gaps() {
    let schema = json!({"methods": [{"name": "prompt.answer"}, {"name": "agent.note"}],
        "x-events": [{"name": "prompt.opened"}], "x-notifications": [{"name": "event"}],
        "components": {"schemas": {"PromptId": {}, "Capabilities": {"properties": {"transports": {}}}}}});
    let mut contract = Contract::of_schema(&schema);
    contract.served = BTreeSet::from(["prompt.answer".to_owned()]);
    contract.fixtures = BTreeSet::from(["fixture:prompt-cards".to_owned()]);
    let guide = parsed(
        &(row("a.ts", "replace", &[]) + &example("e", &json!([call("prompt.answer", &json!({}))]))),
    );
    assert_eq!(contract.report(&guide), contract::consistent());
    contract.served.insert("agent.note".to_owned());
    contract.served.remove("prompt.answer");
    let drifted = contract.report(&guide);
    assert_eq!(
        at(&drifted, "/methods_not_served"),
        json!(["prompt.answer"])
    );
    assert_eq!(at(&drifted, "/unserved_now_served"), json!(["agent.note"]));
}
