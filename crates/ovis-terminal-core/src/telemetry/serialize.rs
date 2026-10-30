use serde_json::{Map, Value};

use super::sink::TelemetryError;
use super::{FieldValue, TraceContext};

const MAX_FIELD_CHARS: usize = 256;

pub(super) fn serialize(
    context: &TraceContext,
    value: Option<FieldValue<'_>>,
) -> Result<Vec<u8>, TelemetryError> {
    let mut fields = Map::new();
    if let Some(request) = &context.request {
        insert_bounded_text(&mut fields, "request", request.as_ref());
    }
    if let Some(operation) = &context.operation {
        insert_bounded_text(&mut fields, "operation", operation.as_ref());
    }
    if let Some(principal) = &context.principal {
        insert_bounded_text(&mut fields, "principal", principal.as_ref());
    }
    if let Some(machine) = &context.machine {
        insert_bounded_text(&mut fields, "machine", machine.as_ref());
    }
    if let Some(runtime) = &context.runtime {
        insert_bounded_text(&mut fields, "runtime", runtime.as_ref());
    }
    if let Some(resource) = &context.resource {
        insert_bounded_text(&mut fields, "resource", resource.as_ref());
    }
    if let Some(sequence) = context.sequence {
        fields.insert("sequence".to_owned(), Value::String(sequence.to_string()));
    }
    if let Some(outcome) = context.outcome {
        fields.insert(
            "outcome".to_owned(),
            Value::String(outcome.field().to_owned()),
        );
    }
    if let Some(value) = value {
        match value {
            FieldValue::Text(text) => insert_bounded_text(&mut fields, "value", text),
            FieldValue::Credential(_) | FieldValue::SecretInput(_) => {
                fields.insert("value".to_owned(), Value::String("[REDACTED]".to_owned()));
            }
        }
    }
    serde_json::to_vec(&Value::Object(fields)).map_err(TelemetryError::TelemetrySerialization)
}

fn insert_bounded_text(fields: &mut Map<String, Value>, key: &str, text: &str) {
    let value: String = text.chars().take(MAX_FIELD_CHARS).collect();
    fields.insert(key.to_owned(), Value::String(value));
}
