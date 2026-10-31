//! Closed label mutation inputs.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::registry::TerminalId;

/// Adds and removes label keys without replacing unrelated metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct LabelRequest {
    /// Explicit terminal identity.
    pub terminal: TerminalId,
    /// Keys to set; setting a key takes precedence over removing it in this request.
    pub set: BTreeMap<String, String>,
    /// Keys to remove.
    pub remove: Vec<String>,
}

impl LabelRequest {
    /// Constructs a label change; key syntax is checked at mutation admission.
    pub const fn new(
        terminal: TerminalId,
        set: BTreeMap<String, String>,
        remove: Vec<String>,
    ) -> Self {
        Self {
            terminal,
            set,
            remove,
        }
    }
}
