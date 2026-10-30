//! `doc.*`: versioned opaque JSON documents with compare-and-swap.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::OpId;
use crate::schema::common::{MachineRef, PageInfo, PageRequest};
use crate::schema::ids::DocName;
use crate::schema::method::{MethodSpec, method};
use crate::schema::scalar::{EventSeq, RecordVersion};

/// Parameters of `doc.get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DocGetParams {
    /// Document name.
    pub name: DocName,
    /// Machine that stores the document.
    pub machine: Option<MachineRef>,
}

/// A document; an absent document is `{value: null, version: "0"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct DocRecord {
    /// Document name.
    pub name: DocName,
    /// Opaque JSON value, or null.
    pub value: Value,
    /// Version; kept when the value is deleted.
    pub version: RecordVersion,
}

/// Parameters of `doc.set`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DocSetParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Document name.
    pub name: DocName,
    /// New value; null deletes.
    pub value: Value,
    /// Version the caller read.
    pub version: RecordVersion,
    /// Machine that stores the document.
    pub machine: Option<MachineRef>,
}

/// Result of `doc.set`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct DocSetResult {
    /// New version.
    pub version: RecordVersion,
    /// Sequence of the change event.
    pub seq: EventSeq,
}

/// Parameters of `doc.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DocListParams {
    /// Only names with this prefix.
    pub prefix: Option<String>,
    /// Page request.
    pub page: Option<PageRequest>,
}

/// Name and version of one document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct DocEntry {
    /// Document name.
    pub name: DocName,
    /// Version.
    pub version: RecordVersion,
}

/// Result of `doc.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct DocListResult {
    /// Documents.
    pub docs: Vec<DocEntry>,
    /// Paging facts.
    pub page: PageInfo,
}

/// Methods of this namespace.
pub(crate) const METHODS: &[MethodSpec] = &[
    method! {"doc.get", Query, Untargeted, DocGetParams => DocRecord,
    "Read a document; absence is version zero with a null value.", [InvalidName, MachineUnreachable]},
    method! {"doc.set", Mutation, Untargeted, DocSetParams => DocSetResult,
    "Compare-and-swap a document in one transaction with its event.", [InvalidName, Conflict, TooLarge, MachineUnreachable]},
    method! {"doc.list", Query, Untargeted, DocListParams => DocListResult,
    "List document names and versions.", []},
];
