//! Comparison state shared by the input and output walks, and their entry point.

use serde_json::Value;

use super::facts::Alt;
use super::{Break, BreakRule, input, output};
use crate::schema::error::SchemaError;

/// Deepest nesting of schemas and references compared before failing with a typed error.
pub(super) const MAX_DEPTH: usize = 64;

/// Whether a schema describes what clients send or what they receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Position {
    /// Clients send it; the new schema must accept everything the old one did.
    Input,
    /// Clients receive it; the new schema must not produce what the old one could not.
    Output,
}

/// The released and current documents with one schema from each.
#[derive(Debug, Clone, Copy)]
pub(super) struct SchemaPair<'a> {
    pub(super) old_doc: &'a Value,
    pub(super) new_doc: &'a Value,
    pub(super) old_value: &'a Value,
    pub(super) new_value: &'a Value,
}

impl<'a> SchemaPair<'a> {
    pub(super) const fn documents(old_doc: &'a Value, new_doc: &'a Value) -> Self {
        Self {
            old_doc,
            new_doc,
            old_value: old_doc,
            new_value: new_doc,
        }
    }

    pub(super) const fn with(self, old_value: &'a Value, new_value: &'a Value) -> Self {
        Self {
            old_value,
            new_value,
            ..self
        }
    }
}

/// References already entered on the way to a schema, and its nesting depth.
#[derive(Debug, Clone, Default)]
pub(super) struct Trail {
    pub(super) refs: Vec<String>,
    pub(super) depth: usize,
}

impl Trail {
    /// Counts one more level of nesting.
    ///
    /// # Errors
    ///
    /// Returns [`SchemaError::DepthExceeded`] beyond [`MAX_DEPTH`].
    pub(super) fn deeper(&mut self, location: &str) -> Result<(), SchemaError> {
        self.depth = self.depth.saturating_add(1);
        if self.depth > MAX_DEPTH {
            return Err(SchemaError::DepthExceeded {
                location: location.to_owned(),
            });
        }
        Ok(())
    }
}

/// Where in a method, event or notification the comparison currently is.
#[derive(Debug, Clone)]
pub(super) struct Site {
    pub(super) position: Position,
    pub(super) path: String,
    pub(super) old: Trail,
    pub(super) new: Trail,
}

impl Site {
    /// The site of a member of the alternatives `old` and `new`.
    pub(super) fn descend(&self, suffix: &str, (old, new): (&Alt<'_>, &Alt<'_>)) -> Self {
        Self {
            position: self.position,
            path: format!("{}{suffix}", self.path),
            old: old.trail.clone(),
            new: new.trail.clone(),
        }
    }

    pub(super) fn report(&self, rule: BreakRule, out: &mut Vec<Break>) {
        out.push(Break {
            path: self.path.clone(),
            rule,
        });
    }
}

/// Lists the breaks between one released schema and its current form.
///
/// # Errors
///
/// Returns [`SchemaError::ReferenceCycle`], [`SchemaError::DepthExceeded`] or
/// [`SchemaError::UnresolvedReference`] for a schema that cannot be walked safely.
pub(super) fn compare(
    pair: SchemaPair<'_>,
    position: Position,
    path: &str,
) -> Result<Vec<Break>, SchemaError> {
    let site = Site {
        position,
        path: path.to_owned(),
        old: Trail::default(),
        new: Trail::default(),
    };
    match position {
        Position::Input => input::breaks(pair, &site),
        Position::Output => output::breaks(pair, &site),
    }
}
