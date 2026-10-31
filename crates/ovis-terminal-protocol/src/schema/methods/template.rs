//! `template.*`: versioned launch templates discoverable by name.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::template_launch::ReadinessContract;
use crate::schema::common::NoParams;
use crate::schema::ids::TemplateName;
use crate::schema::method::{MethodSpec, method};
use crate::schema::scalar::DurationMs;

/// Versioned launch template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TemplateRecord {
    /// Template name.
    pub name: TemplateName,
    /// Template version.
    pub version: String,
    /// Description.
    pub description: String,
    /// Argument vector.
    pub argv: Vec<String>,
    /// Environment the program starts with, before launch additions.
    pub env: BTreeMap<String, String>,
    /// How the program declares it can take its first message.
    pub readiness: ReadinessContract,
    /// Readiness deadline of a first message whose launch names none.
    pub ready_deadline_ms: DurationMs,
    /// Text placed before the task in the first message.
    pub body: Option<String>,
}

/// Parameters of `template.get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TemplateGetParams {
    /// Template name.
    pub name: TemplateName,
    /// Version; absent selects the latest.
    pub version: Option<String>,
}

/// Result of `template.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TemplateListResult {
    /// Templates.
    pub templates: Vec<TemplateRecord>,
}

/// Methods of this namespace.
pub(crate) const METHODS: &[MethodSpec] = &[
    method! {"template.list", Query, Untargeted, NoParams => TemplateListResult,
    "List launch templates.", [TemplateInvalid]},
    method! {"template.get", Query, Lookup, TemplateGetParams => TemplateRecord,
    "Read one launch template.", [TemplateNotFound, TemplateInvalid]},
];
