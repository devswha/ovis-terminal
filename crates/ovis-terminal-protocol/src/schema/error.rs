//! Typed failures of schema generation, compatibility gating and advertisement.

use thiserror::Error;

use super::compat::Break;

/// Failure while generating, checking or advertising the published schema.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SchemaError {
    /// The current schema breaks clients of the released baseline.
    #[error("{} change(s) break clients of the released schema", breaks.len())]
    Incompatible {
        /// Every breaking change found.
        breaks: Vec<Break>,
    },
    /// Two published definitions would share one name.
    #[error("definition name {name} is generated for two different types")]
    NameCollision {
        /// The colliding name.
        name: String,
    },
    /// A method, event or notification name appears twice.
    #[error("published name {name} appears more than once")]
    DuplicateName {
        /// The duplicated name.
        name: String,
    },
    /// A reference points at no definition.
    #[error("reference {reference} resolves to no definition")]
    UnresolvedReference {
        /// The dangling reference.
        reference: String,
    },
    /// A registered handler names a method the catalog does not specify.
    #[error("method {method} is registered but absent from the catalog")]
    UncataloguedMethod {
        /// The registered method.
        method: String,
    },
    /// A document lacks a section the gate needs.
    #[error("schema document lacks {section}")]
    Malformed {
        /// The missing section.
        section: &'static str,
    },
    /// The document could not be encoded or decoded as JSON.
    #[error("schema document is not valid JSON")]
    Json {
        /// Underlying JSON failure.
        #[source]
        source: serde_json::Error,
    },
}

impl SchemaError {
    /// Returns the stable machine token of this failure.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Incompatible { .. } => "schema_incompatible",
            Self::NameCollision { .. } => "schema_name_collision",
            Self::DuplicateName { .. } => "schema_duplicate_name",
            Self::UnresolvedReference { .. } => "schema_unresolved_reference",
            Self::UncataloguedMethod { .. } => "method_not_cataloged",
            Self::Malformed { .. } => "schema_malformed",
            Self::Json { .. } => "schema_json",
        }
    }

    /// Returns recovery guidance for this failure.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Incompatible { .. } => {
                "keep released methods and required outputs; add inputs only as optional"
            }
            Self::NameCollision { .. } => "give each wire type a namespace-prefixed name",
            Self::DuplicateName { .. } => "publish each method and event name once",
            Self::UnresolvedReference { .. } => "derive the referenced type's schema",
            Self::UncataloguedMethod { .. } => "add the method to its namespace catalog first",
            Self::Malformed { .. } | Self::Json { .. } => {
                "regenerate the document with the schema generator"
            }
        }
    }
}

impl From<serde_json::Error> for SchemaError {
    fn from(source: serde_json::Error) -> Self {
        Self::Json { source }
    }
}
