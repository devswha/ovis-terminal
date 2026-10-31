//! Assembly of the deterministic OpenRPC document from the typed catalog.

use schemars::SchemaGenerator;
use schemars::generate::SchemaSettings;
use serde_json::{Value, json};

use super::error::SchemaError;
use super::error_catalog::{CatalogEntry, catalog, method_error};
use super::event_catalog::EVENTS;
use super::events::{EventEnvelope, Heartbeat, MissedNotice};
use super::limits::{Capabilities, Limits};
use super::method::{Effect, MethodSpec, Targeting};
use super::methods::all_methods;
use super::records::{AttachControl, AttachData};
use super::scalar::{OPEN_ENUM_EXTENSION, ProtocolGeneration};
use super::stream::stream_layout;
use super::verify::verify_document;
use crate::errors::ErrorData;

/// OpenRPC specification version the document follows.
pub const OPENRPC_VERSION: &str = "1.3.2";
/// Protocol generation of this build; additive releases keep it.
pub const PROTOCOL_GENERATION: ProtocolGeneration = ProtocolGeneration::new(1);
/// JSON pointer prefix of every shared definition.
pub const COMPONENTS_PREFIX: &str = "#/components/schemas/";

const fn effect_token(effect: Effect) -> &'static str {
    match effect {
        Effect::Query => "query",
        Effect::Mutation => "mutation",
        Effect::Control => "control",
    }
}

const fn targeting_token(targeting: Targeting) -> &'static str {
    match targeting {
        Targeting::Untargeted => "untargeted",
        Targeting::Lookup => "lookup",
        Targeting::Terminal => "terminal",
    }
}

/// Splits a params object definition into by-name OpenRPC content descriptors.
fn param_descriptors(
    reference: &Value,
    generator: &SchemaGenerator,
) -> Result<Vec<Value>, SchemaError> {
    let name = reference
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|target| target.strip_prefix(COMPONENTS_PREFIX))
        .ok_or(SchemaError::Malformed {
            section: "params reference",
        })?;
    let definition =
        generator
            .definitions()
            .get(name)
            .ok_or_else(|| SchemaError::UnresolvedReference {
                reference: name.to_owned(),
            })?;
    let required: Vec<&str> = definition
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let properties = definition.get("properties").and_then(Value::as_object);
    Ok(properties
        .into_iter()
        .flatten()
        .map(|(param, schema)| {
            json!({ "name": param, "required": required.contains(&param.as_str()), "schema": schema })
        })
        .collect())
}

fn method_object(spec: &MethodSpec, generator: &mut SchemaGenerator) -> Result<Value, SchemaError> {
    let params_ref = (spec.params)(generator).to_value();
    let params = param_descriptors(&params_ref, generator)?;
    let errors = spec
        .error_kinds()
        .into_iter()
        .map(|kind| method_error(kind).map(|(kind, code)| json!({ "code": code, "message": kind })))
        .collect::<Result<Vec<Value>, serde_json::Error>>()?;
    Ok(json!({
        "name": spec.name,
        "summary": spec.summary,
        "paramStructure": "by-name",
        "params": params,
        "result": { "name": "result", "schema": (spec.result)(generator).to_value() },
        "errors": errors,
        "x-params": params_ref,
        "x-effect": effect_token(spec.effect),
        "x-targeting": targeting_token(spec.targeting),
    }))
}

fn notification(name: &str, schema: &Value) -> Value {
    json!({ "name": name, "schema": schema })
}

/// Builds the OpenRPC document of the full v1 catalog and verifies it.
///
/// # Errors
///
/// Returns [`SchemaError`] when a name is duplicated, a reference dangles,
/// a definition name collides or a value cannot be encoded.
pub fn openrpc_document() -> Result<Value, SchemaError> {
    let mut generator = SchemaSettings::draft2020_12()
        .with(|settings| {
            settings.definitions_path = COMPONENTS_PREFIX.into();
            settings.meta_schema = None;
        })
        .into_generator();
    let methods = all_methods()
        .map(|spec| method_object(spec, &mut generator))
        .collect::<Result<Vec<Value>, SchemaError>>()?;
    let events: Vec<Value> = EVENTS
        .iter()
        .map(|spec| json!({ "name": spec.name, "summary": spec.summary, "schema": (spec.data)(&mut generator).to_value() }))
        .collect();
    let notifications = vec![
        notification(
            "event",
            generator.subschema_for::<EventEnvelope>().as_value(),
        ),
        notification(
            "heartbeat",
            generator.subschema_for::<Heartbeat>().as_value(),
        ),
        notification(
            "missed",
            generator.subschema_for::<MissedNotice>().as_value(),
        ),
        notification(
            "attach.data",
            generator.subschema_for::<AttachData>().as_value(),
        ),
        notification(
            "attach.control",
            generator.subschema_for::<AttachControl>().as_value(),
        ),
    ];
    let error_kinds: Vec<Value> = catalog(&mut generator)
        .iter()
        .map(CatalogEntry::to_json)
        .collect();
    let document = json!({
        "openrpc": OPENRPC_VERSION,
        "info": {
            "title": "ovis-terminal",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Headless terminal runtime for coding agents. JSON-RPC 2.0 over persistent NDJSON, upgradable to framed channels after hello. Errors carry a stable data.kind.",
            "license": { "name": "MIT" },
            "x-generation": PROTOCOL_GENERATION,
        },
        "methods": methods,
        "x-events": events,
        "x-notifications": notifications,
        "x-error-data": generator.subschema_for::<ErrorData>().to_value(),
        "x-error-kinds": error_kinds,
        "x-limits": { "schema": generator.subschema_for::<Limits>().to_value(), "defaults": Limits::v1() },
        "x-capabilities": { "schema": generator.subschema_for::<Capabilities>().to_value() },
        "x-stream": stream_layout(),
        "x-compatibility": {
            "generation": PROTOCOL_GENERATION,
            "inputs": "closed objects; new inputs are optional; accepted values never narrow",
            "outputs": "fields and values may be added; required fields are never removed",
            "open_enum_extension": OPEN_ENUM_EXTENSION,
            "unknown_enum_value": "decode an unrecognized value of an open enum as an unknown value that keeps the original string",
        },
        "components": { "schemas": generator.take_definitions(true) },
    });
    verify_document(&document)?;
    Ok(document)
}

/// Renders the document as pretty JSON with a trailing newline.
///
/// # Errors
///
/// Returns [`SchemaError`] when the document cannot be built or encoded.
pub fn render_openrpc() -> Result<String, SchemaError> {
    let mut rendered = serde_json::to_string_pretty(&openrpc_document()?)?;
    rendered.push('\n');
    Ok(rendered)
}
