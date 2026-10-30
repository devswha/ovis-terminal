//! `template.*`: versioned launch templates discoverable by name.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::schema::common::NoParams;
use crate::schema::ids::TemplateName;
use crate::schema::method::{MethodSpec, method};

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
