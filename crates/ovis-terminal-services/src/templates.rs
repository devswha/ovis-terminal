//! Versioned launch templates discoverable by stable name and version (plan todo 35).
//!
//! A template is one closed JSON definition in the catalog directory: a name, a
//! dotted numeric version, a literal argument vector, an environment, a declared
//! readiness contract and an optional first-message body. Definitions are read fresh
//! on every request. A template resolves to the same [`LaunchPlan`] a direct argument
//! vector does; task text only ever becomes part of the first message, never argv.

mod catalog;
mod definition;
mod error;
mod plan;
#[cfg(test)]
mod tests;

pub use catalog::TemplateCatalog;
pub use definition::{Template, TemplateVersion};
pub use error::{DefinitionError, TemplateError};
pub use plan::{LaunchPlan, PlannedMessage};
