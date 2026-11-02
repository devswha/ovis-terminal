use ovis_terminal_protocol::schema::method::Effect;
use ovis_terminal_protocol::schema::methods::all_methods;
use ovis_terminal_protocol::schema::openrpc_document;
use serde_json::Value;

use super::VERBS;
use crate::commands::{SERVER, TargetUse};

fn params_schema<'a>(document: &'a Value, method: &str) -> &'a Value {
    let reference = document
        .get("methods")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|entry| entry.get("name").and_then(Value::as_str) == Some(method))
        .and_then(|entry| entry.pointer("/x-params/$ref"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let name = reference.rsplit('/').next().unwrap_or_default();
    document
        .pointer(&format!("/components/schemas/{name}"))
        .unwrap_or(&Value::Null)
}

fn named(schema: &Value, key: &str, field: &str) -> bool {
    match schema.get(key) {
        Some(Value::Array(fields)) => fields.iter().any(|item| item == field),
        Some(Value::Object(fields)) => fields.contains_key(field),
        _ => false,
    }
}

#[test]
fn every_verb_takes_exactly_the_flags_its_closed_params_type_takes() {
    let document = openrpc_document().expect("the schema renders");
    for verb in VERBS.into_iter().chain(SERVER) {
        let spec = all_methods()
            .find(|spec| spec.name == verb.method)
            .unwrap_or_else(|| panic!("{} is not published", verb.method));
        let schema = params_schema(&document, verb.method);
        let takes_op = named(schema, "properties", "op_id");
        assert_eq!(
            spec.effect == Effect::Mutation,
            takes_op,
            "{}: --op-id must exist exactly for mutations",
            verb.method
        );
        assert_eq!(
            verb.token,
            named(schema, "properties", "process_token"),
            "{}: process token use",
            verb.method
        );
        let target = match (
            named(schema, "properties", "target"),
            named(schema, "required", "target"),
        ) {
            (true, true) => TargetUse::Required,
            (true, false) => TargetUse::Optional,
            (false, _) => TargetUse::Absent,
        };
        assert_eq!(verb.target, target, "{}: target use", verb.method);
        if target == TargetUse::Required {
            // The targetless check decodes a stand-in address as this field.
            assert_eq!(
                schema.pointer("/properties/target/$ref"),
                Some(&Value::from("#/components/schemas/Target")),
                "{}: a required target is the common Target",
                verb.method
            );
        }
    }
}
