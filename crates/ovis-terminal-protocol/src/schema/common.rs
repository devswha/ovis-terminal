//! Shapes shared by many methods: targets, expectations, paging and geometry.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::ids::{MachineId, TerminalAddress, TerminalId, TerminalName};
use super::scalar::{ByteOffset, EventSeq, Incarnation, Revision};

/// Exactly one explicit way to name a terminal; there is no focus fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", try_from = "TargetFields")]
#[non_exhaustive]
pub enum Target {
    /// The stable terminal identity.
    TerminalId(TerminalId),
    /// A terminal address, optionally machine-qualified.
    Address(TerminalAddress),
    /// A unique live terminal name.
    Name(TerminalName),
}

/// Closed field set of a target object, so an unknown key reports its own path.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetFields {
    terminal_id: Option<TerminalId>,
    address: Option<TerminalAddress>,
    name: Option<TerminalName>,
}

/// A target object that names zero or several terminals at once.
#[derive(Debug, Error)]
#[error("a target names exactly one of terminal_id, address or name")]
struct TargetArityError;

impl TryFrom<TargetFields> for Target {
    type Error = TargetArityError;

    fn try_from(fields: TargetFields) -> Result<Self, Self::Error> {
        match (fields.terminal_id, fields.address, fields.name) {
            (Some(id), None, None) => Ok(Self::TerminalId(id)),
            (None, Some(address), None) => Ok(Self::Address(address)),
            (None, None, Some(name)) => Ok(Self::Name(name)),
            _ => Err(TargetArityError),
        }
    }
}

impl JsonSchema for Target {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Target")
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let branch = |key: &str, schema: Schema| {
            json_schema!({
                "type": "object",
                "properties": { key: schema },
                "required": [key],
                "additionalProperties": false,
            })
        };
        json_schema!({
            "description": "Exactly one explicit way to name a terminal; there is no focus fallback.",
            "oneOf": [
                branch("terminal_id", generator.subschema_for::<TerminalId>()),
                branch("address", generator.subschema_for::<TerminalAddress>()),
                branch("name", generator.subschema_for::<TerminalName>()),
            ],
        })
    }
}

/// Identities the caller expects at admission; any mismatch writes nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Expect {
    /// Expected terminal incarnation.
    pub terminal_incarnation: Option<Incarnation>,
    /// Expected program incarnation.
    pub process_incarnation: Option<Incarnation>,
    /// Expected latest status sequence.
    pub status_seq: Option<EventSeq>,
}

/// Request for one page of a listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PageRequest {
    /// Opaque continuation returned by the previous page.
    pub after: Option<PageToken>,
    /// Maximum number of entries in this page.
    pub limit: Option<u32>,
}

/// Opaque continuation token bound to one pinned snapshot revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct PageToken(String);

impl PageToken {
    /// Wraps a continuation issued by a listing.
    #[must_use]
    pub const fn new(token: String) -> Self {
        Self(token)
    }

    /// Borrows the continuation text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Paging facts returned with every listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct PageInfo {
    /// Snapshot revision all pages of this listing are pinned to.
    pub revision: Revision,
    /// Total number of entries at that revision.
    pub total: u32,
    /// Continuation for the next page, absent on the last page.
    pub next: Option<PageToken>,
    /// Whether the listing was cut short by a size bound.
    pub truncated: bool,
}

/// Terminal grid size in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Geometry {
    /// Number of columns.
    pub cols: u16,
    /// Number of rows.
    pub rows: u16,
}

impl Geometry {
    /// A grid of `cols` columns and `rows` rows.
    #[must_use]
    pub const fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }
}

/// Cursor into the runtime-lifetime event sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EventCursor {
    /// Runtime lifetime the sequence belongs to.
    pub runtime_lifetime: super::ids::RuntimeLifetimeId,
    /// Last sequence the caller has processed.
    pub seq: EventSeq,
}

/// Cursor into retained raw terminal output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct OutputCursor {
    /// Raw output offset the caller has observed.
    pub offset: ByteOffset,
}

/// Text or regular-expression pattern for searches and waits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Pattern {
    /// Literal text.
    Text(String),
    /// Regular expression.
    Regex(String),
}

/// Machine scope of a request; absent means the local machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum MachineRef {
    /// Stable machine identity.
    MachineId(MachineId),
    /// Machine name or alias, resolved once at admission.
    Name(super::ids::MachineName),
}

/// Requested shape of terminal text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TextFormat {
    /// Plain text without escape sequences.
    Text,
    /// Text with SGR escape sequences.
    Ansi,
    /// Structured cells.
    Cells,
}

/// Empty, closed parameter object for methods without parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "params arrive as an empty JSON object, which a unit struct does not decode"
)]
pub struct NoParams {}
