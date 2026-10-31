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
    /// A compared document has a missing section or a malformed entry.
    #[error("compared schema is malformed at {location}")]
    MalformedBaseline {
        /// Where the malformed entry or missing section is.
        location: String,
    },
    /// A reference leads back to itself on one path.
    #[error("reference {reference} leads back to itself")]
    ReferenceCycle {
        /// The reference entered twice.
        reference: String,
    },
    /// A schema nests deeper than the comparison bound.
    #[error("schema nesting exceeds the comparison bound at {location}")]
    DepthExceeded {
        /// Where the bound was exceeded.
        location: String,
    },
    /// A schema composition expands to more alternatives than the comparison bound.
    #[error("schema composition under {location} expands beyond {limit} alternatives")]
    TooManyAlternatives {
        /// The nearest reference above the composition, or the schema root.
        location: String,
        /// The bound on alternatives per schema.
        limit: usize,
    },
    /// The released baseline changed without a ledger entry recording the amendment.
    #[error(
        "released baseline sha256:{digest} has no amendment entry with previous: sha256:{previous}"
    )]
    AmendmentMissing {
        /// Digest of the released baseline as it is now.
        digest: String,
        /// Digest the ledger last records; the missing entry continues from it.
        previous: String,
    },
    /// The baseline amendment ledger lacks a field or breaks its digest chain.
    #[error("baseline amendment ledger is malformed at {location}")]
    AmendmentsMalformed {
        /// The entry and field that is missing or out of chain.
        location: String,
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
            Self::MalformedBaseline { .. } => "malformed_baseline",
            Self::ReferenceCycle { .. } => "schema_reference_cycle",
            Self::DepthExceeded { .. } => "schema_depth_exceeded",
            Self::TooManyAlternatives { .. } => "schema_alternatives_exceeded",
            Self::AmendmentMissing { .. } => "baseline_amendment_missing",
            Self::AmendmentsMalformed { .. } => "baseline_amendments_malformed",
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
            Self::ReferenceCycle { .. } => "replace the cyclic reference with a finite definition",
            Self::DepthExceeded { .. } => "flatten the schema below 64 nested levels",
            Self::TooManyAlternatives { .. } => {
                "split the union into named definitions with fewer combinations"
            }
            Self::AmendmentMissing { .. } => {
                "add an entry to schema/BASELINE-AMENDMENTS.md with date, pr, fields, reason, previous and current digests"
            }
            Self::AmendmentsMalformed { .. } => {
                "give every entry date, pr, fields, reason, previous and current; chain each previous to the prior digest"
            }
            Self::MalformedBaseline { .. } => {
                "regenerate or restore the released baseline; every entry needs a name and known keys"
            }
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
