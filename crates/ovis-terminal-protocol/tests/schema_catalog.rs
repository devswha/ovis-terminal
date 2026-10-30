//! Catalog invariants that the generated document must keep for every method.

use ovis_terminal_protocol::schema::ids::MethodName;
use ovis_terminal_protocol::schema::method::Effect;
use ovis_terminal_protocol::schema::methods::all_methods;
use ovis_terminal_protocol::schema::methods::terminal::TerminalListParams;
use ovis_terminal_protocol::schema::records::TerminalState;
use ovis_terminal_protocol::schema::scalar::{OpenEnum, Revision};
use ovis_terminal_protocol::schema::{advertised_methods, openrpc_document};
use ovis_terminal_protocol::wire::{Channel, Frame, FrameKind, MAX_FRAME_PAYLOAD};
use serde_json::{Value, json};

#[cfg(test)]
fn document() -> Value {
    openrpc_document().expect("the catalog should generate")
}

#[cfg(test)]
fn param<'a>(method: &'a Value, name: &str) -> Option<&'a Value> {
    let params = method.get("params").and_then(Value::as_array)?;
    params
        .iter()
        .find(|p| p.get("name").and_then(Value::as_str) == Some(name))
}

#[cfg(test)]
fn required(param: Option<&Value>) -> bool {
    param
        .and_then(|p| p.get("required"))
        .and_then(Value::as_bool)
        == Some(true)
}

#[test]
fn mutations_require_an_operation_id_and_reads_take_none() {
    let doc = document();
    for method in doc
        .get("methods")
        .and_then(Value::as_array)
        .expect("methods")
    {
        let name = method.get("name").and_then(Value::as_str).expect("name");
        match method.get("x-effect").and_then(Value::as_str) {
            Some("mutation") => assert!(required(param(method, "op_id")), "{name} lacks op_id"),
            Some("query" | "control") => assert!(
                param(method, "op_id").is_none() || name.starts_with("op."),
                "{name} takes op_id"
            ),
            other => panic!("{name} has effect {other:?}"),
        }
        if method.get("x-targeting").and_then(Value::as_str) == Some("terminal") {
            assert!(
                required(param(method, "target")),
                "{name} lacks a required target"
            );
        }
    }
}

/// Collects enum definitions reachable from output schemas and whether each is open.
struct EnumWalk<'a> {
    doc: &'a Value,
    seen: Vec<String>,
    found: Vec<(String, bool)>,
}

#[cfg(test)]
impl<'a> EnumWalk<'a> {
    fn visit(&mut self, schema: &'a Value) {
        let open = schema.get("x-ovis-open-enum") == Some(&Value::Bool(true));
        let reference = schema.get("$ref").and_then(Value::as_str);
        if let Some(name) = reference.and_then(|r| r.strip_prefix("#/components/schemas/")) {
            let key = format!("{name}:{open}");
            if self.seen.contains(&key) {
                return;
            }
            self.seen.push(key);
            let target = self
                .doc
                .pointer(&format!("/components/schemas/{name}"))
                .expect("reference resolves");
            let variants = target.get("oneOf").and_then(Value::as_array);
            let is_enum = target.get("enum").is_some()
                || variants.is_some_and(|m| m.iter().all(|v| v.get("const").is_some()));
            if is_enum {
                let declared_open = target.get("x-ovis-open-enum") == Some(&Value::Bool(true));
                self.found.push((name.to_owned(), open || declared_open));
            } else {
                self.visit(target);
            }
            return;
        }
        match schema {
            Value::Object(object) => object.values().for_each(|child| self.visit(child)),
            Value::Array(items) => items.iter().for_each(|child| self.visit(child)),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
}

#[test]
fn every_output_enum_tolerates_additions() {
    let doc = document();
    let methods = doc
        .get("methods")
        .and_then(Value::as_array)
        .expect("methods");
    let events = doc
        .get("x-events")
        .and_then(Value::as_array)
        .expect("events");
    let outputs = methods
        .iter()
        .filter_map(|m| m.pointer("/result/schema"))
        .chain(events.iter().filter_map(|e| e.get("schema")));
    let mut walk = EnumWalk {
        doc: &doc,
        seen: Vec::new(),
        found: Vec::new(),
    };
    for schema in outputs {
        walk.visit(schema);
    }
    let closed: Vec<&String> = walk
        .found
        .iter()
        .filter(|(_, open)| !open)
        .map(|(name, _)| name)
        .collect();
    assert!(!walk.found.is_empty(), "outputs carry enums");
    assert!(
        closed.is_empty(),
        "closed enums in output position: {closed:?}"
    );
}

#[test]
fn unknown_output_enum_value_decodes_as_unknown() {
    let future: OpenEnum<TerminalState> =
        serde_json::from_value(json!("hibernating")).expect("decodes");
    assert_eq!(future, OpenEnum::Unknown("hibernating".to_owned()));
    let known: OpenEnum<TerminalState> = serde_json::from_value(json!("lost")).expect("decodes");
    assert_eq!(known, OpenEnum::Known(TerminalState::Lost));
}

#[test]
fn input_enums_stay_closed() {
    let result = serde_json::from_value::<TerminalListParams>(json!({ "states": ["hibernating"] }));
    assert!(result.is_err(), "an unknown input value must be rejected");
}

#[test]
fn hello_advertises_only_registered_methods_in_catalog_order() {
    let advertised = advertised_methods(["terminal.get", "hello"]).expect("both are catalogued");
    let names: Vec<&str> = advertised.iter().map(MethodName::as_str).collect();
    assert_eq!(names, ["hello", "terminal.get"]);
    assert!(
        all_methods().count() > names.len(),
        "specified methods exceed implemented ones"
    );
    let error = advertised_methods(["pane.get"]).expect_err("an uncatalogued handler is refused");
    assert_eq!(error.kind(), "method_not_cataloged");
    let error = advertised_methods(["hello", "hello"]).expect_err("a duplicate handler is refused");
    assert_eq!(error.kind(), "schema_duplicate_name");
}

#[test]
fn catalog_effects_match_published_effects() {
    let doc = document();
    let methods = doc
        .get("methods")
        .and_then(Value::as_array)
        .expect("methods");
    for (spec, method) in all_methods().zip(methods) {
        assert_eq!(method.get("name").and_then(Value::as_str), Some(spec.name));
        let mutation = method.get("x-effect").and_then(Value::as_str) == Some("mutation");
        assert_eq!(mutation, spec.effect == Effect::Mutation, "{}", spec.name);
    }
}

#[test]
fn stream_layout_matches_the_codec() {
    let doc = document();
    let kinds = doc
        .pointer("/x-stream/kinds")
        .and_then(Value::as_array)
        .expect("kinds");
    for entry in kinds {
        let value = entry
            .get("value")
            .and_then(Value::as_u64)
            .and_then(|v| u8::try_from(v).ok())
            .expect("u8");
        let kind = FrameKind::try_from(value).expect("documented kinds decode");
        let encoded = Frame::new(Channel::new(7), kind, vec![1, 2, 3])
            .expect("frame")
            .encode()
            .expect("encodes");
        assert_eq!(
            encoded.get(4).copied(),
            Some(value),
            "kind byte at the documented offset"
        );
        assert_eq!(
            encoded.get(5..9),
            Some(&3_u32.to_le_bytes()[..]),
            "length at the documented offset"
        );
        assert_eq!(
            encoded.get(0..4),
            Some(&7_u32.to_le_bytes()[..]),
            "channel at the documented offset"
        );
    }
    let max = doc
        .pointer("/x-stream/max_payload_bytes")
        .and_then(Value::as_u64);
    assert_eq!(max, u64::try_from(MAX_FRAME_PAYLOAD).ok());
    let advertised = doc
        .pointer("/x-limits/defaults/max_frame_bytes")
        .and_then(Value::as_str);
    assert_eq!(
        advertised,
        Some(MAX_FRAME_PAYLOAD.to_string().as_str()),
        "limits agree with the codec"
    );
}

#[test]
fn counters_round_trip_u64_max_as_decimal_strings() {
    let encoded = serde_json::to_value(Revision::new(u64::MAX)).expect("encodes");
    assert_eq!(encoded, json!("18446744073709551615"));
    let decoded: Revision = serde_json::from_value(encoded).expect("decodes");
    assert_eq!(decoded.get(), u64::MAX);
    assert!(
        serde_json::from_value::<Revision>(json!(5)).is_err(),
        "a JSON number is rejected"
    );
}
