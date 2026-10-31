//! One closed template definition, checked before it can be listed or launched.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;

use ovis_terminal_protocol::domain::template_launch::{DEFAULT_READY_DEADLINE, ReadinessContract};
use ovis_terminal_protocol::schema::ids::TemplateName;
use ovis_terminal_protocol::schema::methods::template::TemplateRecord;
use ovis_terminal_protocol::schema::scalar::DurationMs;
use serde::Deserialize;

use super::DefinitionError;

/// A dotted numeric version; later versions compare greater component by component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateVersion {
    text: String,
    parts: Vec<u64>,
}

impl TemplateVersion {
    /// Parses `1`, `2.1`, `3.0.7`: one to four dot-separated unsigned numbers.
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Version`] for anything else.
    pub fn parse(text: &str) -> Result<Self, DefinitionError> {
        let parts = text
            .split('.')
            .map(|part| {
                if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(DefinitionError::Version);
                }
                part.parse::<u64>().map_err(|_| DefinitionError::Version)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if parts.len() > 4 {
            return Err(DefinitionError::Version);
        }
        Ok(Self {
            text: text.to_owned(),
            parts,
        })
    }

    /// The version as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl Ord for TemplateVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.parts
            .cmp(&other.parts)
            .then_with(|| self.text.cmp(&other.text))
    }
}

impl PartialOrd for TemplateVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for TemplateVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTemplate {
    name: String,
    version: String,
    description: String,
    argv: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    readiness: Option<ReadinessContract>,
    ready_deadline_ms: Option<DurationMs>,
    body: Option<String>,
}

/// A checked template: literal argv, environment, readiness contract and body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub(super) name: TemplateName,
    pub(super) version: TemplateVersion,
    pub(super) description: String,
    pub(super) argv: Vec<String>,
    pub(super) env: BTreeMap<String, String>,
    pub(super) readiness: ReadinessContract,
    pub(super) ready_deadline: DurationMs,
    pub(super) body: Option<String>,
}

impl Template {
    /// Parses and checks one definition file's bytes.
    ///
    /// # Errors
    ///
    /// The [`DefinitionError`] naming the first refused part.
    pub fn parse(bytes: &[u8]) -> Result<Self, DefinitionError> {
        let raw: RawTemplate = serde_json::from_slice(bytes).map_err(DefinitionError::Shape)?;
        if raw.name.is_empty()
            || raw.name.starts_with('.')
            || raw.name.contains(['/', '\\', '@', '\0'])
            || raw.name.chars().any(char::is_whitespace)
        {
            return Err(DefinitionError::Name);
        }
        if raw.argv.first().is_none_or(String::is_empty)
            || raw.argv.iter().any(|arg| arg.contains('\0'))
        {
            return Err(DefinitionError::Argv);
        }
        if raw
            .env
            .iter()
            .any(|(key, value)| key.is_empty() || key.contains(['=', '\0']) || value.contains('\0'))
        {
            return Err(DefinitionError::Environment);
        }
        let readiness = raw
            .readiness
            .unwrap_or_else(ReadinessContract::idle)
            .validated()
            .map_err(DefinitionError::Readiness)?;
        Ok(Self {
            name: TemplateName::new(raw.name),
            version: TemplateVersion::parse(&raw.version)?,
            description: raw.description,
            argv: raw.argv,
            env: raw.env,
            readiness,
            ready_deadline: raw.ready_deadline_ms.unwrap_or(DEFAULT_READY_DEADLINE),
            body: raw.body.filter(|body| !body.is_empty()),
        })
    }

    /// Stable name.
    #[must_use]
    pub const fn name(&self) -> &TemplateName {
        &self.name
    }

    /// Version.
    #[must_use]
    pub const fn version(&self) -> &TemplateVersion {
        &self.version
    }

    /// The published record.
    #[must_use]
    pub fn record(&self) -> TemplateRecord {
        let mut record = TemplateRecord::new(
            (self.name.clone(), self.version.to_string()),
            self.description.clone(),
            self.argv.clone(),
        );
        record.env.clone_from(&self.env);
        record.readiness = self.readiness.clone();
        record.ready_deadline_ms = self.ready_deadline;
        record.body.clone_from(&self.body);
        record
    }
}
