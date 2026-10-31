//! The template directory, read fresh on every request.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use ovis_terminal_protocol::schema::ids::TemplateName;

use super::{DefinitionError, Template, TemplateError};

/// Largest definition file the catalog reads.
const MAX_DEFINITION_BYTES: u64 = 64 * 1024;
/// Most definition files one catalog holds.
const MAX_DEFINITIONS: usize = 256;

/// Templates defined by the `*.json` files of one directory.
///
/// A missing directory is an empty catalog. One unusable file makes the whole catalog
/// `template_invalid`, naming that file, so a broken role is never silently skipped.
#[derive(Debug, Clone)]
pub struct TemplateCatalog {
    dir: PathBuf,
}

impl TemplateCatalog {
    /// The catalog of `dir`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Every template, ordered by name and then version.
    ///
    /// # Errors
    ///
    /// `template_invalid` for an unreadable directory or an unusable definition.
    pub fn list(&self) -> Result<Vec<Template>, TemplateError> {
        let mut templates = self.load()?;
        templates.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
        Ok(templates)
    }

    /// The template `name` at `version`, or its latest version when none is pinned.
    ///
    /// # Errors
    ///
    /// `template_not_found` when nothing matches; `template_invalid` as for [`Self::list`].
    pub fn get(
        &self,
        name: &TemplateName,
        version: Option<&str>,
    ) -> Result<Template, TemplateError> {
        let named = self
            .load()?
            .into_iter()
            .filter(|template| &template.name == name);
        let found = match version {
            Some(pinned) => named
                .into_iter()
                .find(|template| template.version.as_str() == pinned),
            None => named.max_by(|a, b| a.version.cmp(&b.version)),
        };
        found.ok_or_else(|| TemplateError::NotFound {
            name: name.as_str().to_owned(),
            version: version.map(str::to_owned),
        })
    }

    fn load(&self) -> Result<Vec<Template>, TemplateError> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(TemplateError::Unreadable(error)),
        };
        let mut files = Vec::new();
        for entry in entries {
            let path = entry.map_err(TemplateError::Unreadable)?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                files.push(path);
            }
        }
        files.sort();
        if files.len() > MAX_DEFINITIONS {
            return Err(TemplateError::TooMany);
        }
        let mut templates: Vec<Template> = Vec::with_capacity(files.len());
        for file in files {
            let template = read(&file).map_err(|source| TemplateError::Invalid {
                file: file.clone(),
                source,
            })?;
            if templates
                .iter()
                .any(|seen| seen.name == template.name && seen.version == template.version)
            {
                return Err(TemplateError::Invalid {
                    file,
                    source: DefinitionError::Duplicate,
                });
            }
            templates.push(template);
        }
        Ok(templates)
    }
}

fn read(file: &Path) -> Result<Template, DefinitionError> {
    let mut bytes = Vec::new();
    std::fs::File::open(file)
        .and_then(|handle| {
            handle
                .take(MAX_DEFINITION_BYTES.saturating_add(1))
                .read_to_end(&mut bytes)
        })
        .map_err(|error| DefinitionError::Shape(serde_json::Error::io(error)))?;
    if u64::try_from(bytes.len()).map_or(true, |len| len > MAX_DEFINITION_BYTES) {
        return Err(DefinitionError::TooLarge);
    }
    Template::parse(&bytes)
}
