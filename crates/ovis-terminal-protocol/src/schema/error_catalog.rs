//! The published error catalog and the schema of JSON-RPC error data.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde_json::{Value, json};

use super::error_kinds::DomainErrorKind;
use super::method::MethodErrorKind;
use super::scalar::OPEN_ENUM_EXTENSION;
use crate::errors::{BoundaryError, ErrorData, ErrorKind, FieldPath, JsonRpcCode, OpId, TargetId};

/// Envelope kinds produced by request parsing, correlation and dispatch.
const ENVELOPE_KINDS: [ErrorKind; 11] = [
    ErrorKind::ParseError,
    ErrorKind::InvalidRequest,
    ErrorKind::MethodNotFound,
    ErrorKind::InvalidParams,
    ErrorKind::InvalidResponse,
    ErrorKind::DuplicateRequestId,
    ErrorKind::UnknownOrSettledRequestId,
    ErrorKind::ResponseIdMismatch,
    ErrorKind::NotFound,
    ErrorKind::Unavailable,
    ErrorKind::Internal,
];

/// One catalog entry: token, JSON-RPC code and meaning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CatalogEntry {
    kind: String,
    code: i32,
    description: String,
}

impl CatalogEntry {
    pub(super) fn to_json(&self) -> Value {
        json!({ "kind": self.kind, "code": self.code, "description": self.description })
    }
}

fn envelope_entry(kind: ErrorKind) -> CatalogEntry {
    let (token, description) = (kind.token().to_owned(), kind.hint().to_owned());
    let error = BoundaryError::domain(kind, false);
    CatalogEntry {
        kind: token,
        code: JsonRpcCode::from(&error).value(),
        description,
    }
}

/// Domain kinds with the description each variant publishes in its schema.
fn domain_entries(generator: &mut SchemaGenerator) -> Vec<CatalogEntry> {
    let schema = DomainErrorKind::json_schema(generator);
    let variants = schema.get("oneOf").and_then(Value::as_array);
    variants
        .into_iter()
        .flatten()
        .filter_map(|variant| {
            let kind = variant.get("const")?.as_str()?;
            let description = variant.get("description")?.as_str()?;
            Some(CatalogEntry {
                kind: kind.to_owned(),
                code: JsonRpcCode::ServerError.value(),
                description: description.to_owned(),
            })
        })
        .collect()
}

/// Every published error kind: envelope kinds first, then domain kinds.
pub(super) fn catalog(generator: &mut SchemaGenerator) -> Vec<CatalogEntry> {
    let mut entries: Vec<CatalogEntry> = ENVELOPE_KINDS.into_iter().map(envelope_entry).collect();
    entries.extend(domain_entries(generator));
    entries
}

/// Token and JSON-RPC code of one kind a method may return.
pub(super) fn method_error(kind: MethodErrorKind) -> Result<(String, i32), serde_json::Error> {
    match kind {
        MethodErrorKind::Envelope(kind) => {
            let entry = envelope_entry(kind);
            Ok((entry.kind, entry.code))
        }
        MethodErrorKind::Domain(kind) => {
            let token: String = serde_json::from_value(serde_json::to_value(kind)?)?;
            Ok((token, JsonRpcCode::ServerError.value()))
        }
    }
}

/// Schema name of the open error-kind token set.
struct ErrorKindName;

impl JsonSchema for ErrorKindName {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("ErrorKindName")
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let tokens: Vec<String> = catalog(generator)
            .into_iter()
            .map(|entry| entry.kind)
            .collect();
        let mut schema = json_schema!({
            "type": "string",
            "description": "Stable error kind; clients keep unrecognized kinds as unknown values.",
            "enum": tokens,
        });
        schema.insert(OPEN_ENUM_EXTENSION.to_owned(), true.into());
        schema
    }
}

impl JsonSchema for ErrorData {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("ErrorData")
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "object",
            "description": "Machine-readable context of a JSON-RPC error.",
            "properties": {
                "kind": generator.subschema_for::<ErrorKindName>(),
                "hint": { "type": "string", "description": "Recovery guidance." },
                "retryable": { "type": "boolean", "description": "Whether a retry may succeed." },
                "field_path": generator.subschema_for::<FieldPath>(),
                "target": generator.subschema_for::<TargetId>(),
                "op_id": generator.subschema_for::<OpId>(),
                "deadline_ms": generator.subschema_for::<super::scalar::DurationMs>(),
                "elapsed_ms": generator.subschema_for::<super::scalar::DurationMs>(),
                "current_seq": generator.subschema_for::<super::scalar::ReportSeq>(),
                "candidates": generator.subschema_for::<Vec<crate::errors::TargetCandidate>>(),
                "affected": generator.subschema_for::<Vec<TargetId>>(),
            },
            "required": ["kind", "hint", "retryable"],
        })
    }
}
