//! Template lookup and definition failures.

use std::path::PathBuf;

use ovis_terminal_protocol::domain::template_launch::TemplateLaunchError;
use thiserror::Error;

/// Why one definition file is not a usable template.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DefinitionError {
    /// The file is not the closed definition shape.
    #[error("the definition does not match the closed template shape")]
    Shape(#[source] serde_json::Error),
    /// The name is empty, hidden or contains a path separator.
    #[error("the template name must be a plain non-empty word")]
    Name,
    /// The version is not dotted unsigned numbers.
    #[error("the template version must be dotted unsigned numbers")]
    Version,
    /// The argument vector is empty or holds a NUL byte.
    #[error("the argument vector must name a program and hold no NUL byte")]
    Argv,
    /// An environment name is empty or holds `=` or NUL, or a value holds NUL.
    #[error("an environment entry is not a valid name and value")]
    Environment,
    /// The readiness contract is not a declared, typing-safe one.
    #[error("the readiness contract is refused")]
    Readiness(#[source] TemplateLaunchError),
    /// Another file defines the same name and version.
    #[error("another definition has the same name and version")]
    Duplicate,
    /// The file is larger than a definition may be.
    #[error("the definition exceeds 64 KiB")]
    TooLarge,
}

/// A template request the catalog cannot answer.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TemplateError {
    /// No definition has this name and version.
    #[error("no template matches the requested name and version")]
    NotFound {
        /// Requested name.
        name: String,
        /// Requested version, when pinned.
        version: Option<String>,
    },
    /// A definition file cannot be used.
    #[error("the template definition at {file} is unusable")]
    Invalid {
        /// The offending file.
        file: PathBuf,
        /// What is wrong with it.
        #[source]
        source: DefinitionError,
    },
    /// The catalog directory or a definition file cannot be read.
    #[error("the template catalog cannot be read")]
    Unreadable(#[source] std::io::Error),
    /// The directory holds more definitions than a catalog may.
    #[error("the template directory holds more than 256 definitions")]
    TooMany,
}

impl TemplateError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "template_not_found",
            Self::Invalid { .. } | Self::Unreadable(_) | Self::TooMany => "template_invalid",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "list templates and pin an existing name and version",
            Self::Invalid { .. } => "fix or remove the named template definition",
            Self::Unreadable(_) => "make the template directory readable by the runtime",
            Self::TooMany => "remove unused template definitions",
        }
    }
}
