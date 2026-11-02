//! Ovis policies the guide's examples must keep: prompts are answered by version, never
//! by guessed keys; a send is never repeated under a fresh operation id; terminal or
//! conversation content never becomes a request parameter; uncertain delivery is asked
//! about with `op.get`.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

const KEY_INPUT: [&str; 4] = ["input.send", "input.keys", "input.raw", "input.paste"];
const MUTATIONS: [&str; 7] = [
    "input.send",
    "input.keys",
    "input.raw",
    "input.paste",
    "prompt.answer",
    "terminal.run",
    "layout.edit",
];
/// Reply members that carry program or transcript content rather than runtime facts.
const CONTENT: [&str; 7] = [
    "/result/lines",
    "/result/data_b64",
    "/result/text",
    "/result/matches",
    "/result/records",
    "/result/turns",
    "/result/rows",
];

/// One `call` or `legacy` step of an example: its method, params and expectations.
fn calls(case: &Value) -> Vec<(&str, &Value, &Value)> {
    case.get("steps")
        .and_then(Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .filter_map(|step| step.get("call").or_else(|| step.get("legacy")))
                .map(|call| {
                    (
                        call.get("method")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                        call.get("params").unwrap_or(&Value::Null),
                        call.get("expect").unwrap_or(&Value::Null),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn mentions(params: &Value, symbol: &str) -> bool {
    match params {
        Value::String(text) => text == symbol,
        Value::Array(items) => items.iter().any(|item| mentions(item, symbol)),
        Value::Object(map) => map.values().any(|item| mentions(item, symbol)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

fn guessed_keys(case: &Value) -> bool {
    let methods: Vec<&str> = calls(case).into_iter().map(|(method, ..)| method).collect();
    methods.iter().any(|method| method.starts_with("prompt."))
        && methods
            .iter()
            .any(|method| KEY_INPUT.contains(method) || method.starts_with("pane.send"))
}

fn without_op(params: &Value) -> (Value, Option<&Value>) {
    let mut rest: Map<String, Value> = params.as_object().cloned().unwrap_or_default();
    rest.remove("op_id");
    (Value::Object(rest), params.get("op_id"))
}

/// The same mutation sent twice under two different operation ids.
fn auto_resend(case: &Value) -> bool {
    let sent: Vec<(&str, (Value, Option<&Value>))> = calls(case)
        .into_iter()
        .filter(|(method, ..)| MUTATIONS.contains(method))
        .map(|(method, params, _)| (method, without_op(params)))
        .collect();
    sent.iter().enumerate().any(|(at, (method, (rest, op)))| {
        sent.iter()
            .skip(at.saturating_add(1))
            .any(|(other, (other_rest, other_op))| {
                other == method && other_rest == rest && other_op != op
            })
    })
}

/// A value captured from content and later sent as a parameter.
fn content_as_command(case: &Value) -> bool {
    let mut tainted: BTreeSet<String> = BTreeSet::new();
    for (_, params, expect) in calls(case) {
        if tainted
            .iter()
            .any(|name| mentions(params, &format!("$value:{name}")))
        {
            return true;
        }
        let Some(expect) = expect.as_object() else {
            continue;
        };
        for (pointer, matcher) in expect {
            let captured = matcher.get("$capture").and_then(Value::as_str);
            if let Some(name) = captured
                && CONTENT.iter().any(|content| pointer.starts_with(content))
            {
                tainted.insert(name.to_owned());
            }
        }
    }
    false
}

/// An input send whose operation is then inspected with `op.get`.
fn asks_about_delivery(case: &Value) -> bool {
    let mut sent: Vec<&Value> = Vec::new();
    for (method, params, _) in calls(case) {
        if KEY_INPUT.contains(&method)
            && let Some(op) = params.get("op_id")
        {
            sent.push(op);
        } else if method == "op.get" && params.get("op_id").is_some_and(|op| sent.contains(&op)) {
            return true;
        }
    }
    false
}

/// The policy report over every example; [`kept`] is the report of a compliant guide.
pub(super) fn report(examples: &[(String, Value)]) -> Value {
    let offenders = |rule: fn(&Value) -> bool| -> Vec<&str> {
        examples
            .iter()
            .filter(|(_, case)| rule(case))
            .map(|(id, _)| id.as_str())
            .collect()
    };
    json!({
        "guessed_keys": offenders(guessed_keys),
        "auto_resend": offenders(auto_resend),
        "content_as_command": offenders(content_as_command),
        "unknown_delivery_shown": examples.iter().any(|(_, case)| asks_about_delivery(case)),
    })
}

pub(super) fn kept() -> Value {
    json!({
        "guessed_keys": [],
        "auto_resend": [],
        "content_as_command": [],
        "unknown_delivery_shown": true,
    })
}
