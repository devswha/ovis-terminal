//! Walking the source roots and applying every source rule to each Rust file.

use std::path::{Path, PathBuf};

use proc_macro2::TokenStream;
use serde_json::{Value, json};
use workspace_hygiene::loc::pure_loc;
use workspace_hygiene::walk;

use super::error::QualityError;
use super::report::{Finding, count};
use super::rules::syntax_rules;

/// Limits a run enforces; never looser than the binding contract.
#[derive(Debug, Clone, Copy)]
pub(super) struct SourceLimits {
    pub(super) pure_loc: usize,
    pub(super) params: usize,
}

impl SourceLimits {
    /// The requested limits, refused when either is looser than 250 pure LOC / 3 parameters.
    pub(super) const fn tightened(pure_loc: usize, params: usize) -> Result<Self, QualityError> {
        if pure_loc > workspace_hygiene::MAX_PURE_LOC {
            return Err(QualityError::LimitAboveContract {
                flag: "max-pure-loc",
                value: pure_loc,
                limit: workspace_hygiene::MAX_PURE_LOC,
            });
        }
        if params > workspace_hygiene::MAX_PARAMETERS {
            return Err(QualityError::LimitAboveContract {
                flag: "max-params",
                value: params,
                limit: workspace_hygiene::MAX_PARAMETERS,
            });
        }
        Ok(Self { pure_loc, params })
    }
}

/// Every rule on one source text: a file that does not parse is one syntax finding.
pub(super) fn check_text(path: &str, source: &str, limits: SourceLimits) -> Vec<Finding> {
    let syntax = |line: usize, message: String| {
        vec![Finding::new("syntax", format!("{path}:{line}"), message)]
    };
    let file = match syn::parse_file(source) {
        Ok(file) => file,
        Err(error) => return syntax(error.span().start().line, error.to_string()),
    };
    let tokens = match source.parse::<TokenStream>() {
        Ok(tokens) => tokens,
        Err(error) => return syntax(error.span().start().line, error.to_string()),
    };
    let mut found = Vec::new();
    let lines = pure_loc(tokens);
    if lines > limits.pure_loc {
        let detail = format!("{lines} pure lines of code (limit {})", limits.pure_loc);
        found.push(Finding::new("max-pure-loc", format!("{path}:1"), detail));
    }
    found.extend(syntax_rules(path, &file, limits.params));
    found
}

/// What one walk over the roots saw.
#[derive(Debug, Default)]
pub(super) struct Scanned {
    pub(super) findings: Vec<Finding>,
    files: usize,
    roots: Vec<Value>,
}

impl Scanned {
    pub(super) fn facts(&self) -> Value {
        json!({"files_checked": count(self.files), "roots": self.roots})
    }
}

/// `path` relative to `repo`, with `/` separators, for reports.
pub(super) fn shown(repo: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(repo).unwrap_or(path);
    relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Applies every source rule to every `.rs` file under each root of `repo`.
pub(super) fn scan_roots(
    repo: &Path,
    roots: &[PathBuf],
    limits: SourceLimits,
) -> Result<Scanned, QualityError> {
    let mut scanned = Scanned::default();
    for root in roots {
        let dir = repo.join(root);
        if !dir.is_dir() {
            return Err(QualityError::Io {
                path: dir,
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            });
        }
        let sources = walk::rust_sources(&dir).map_err(|source| QualityError::Walk { source })?;
        for path in &sources {
            let text = walk::read_source(path).map_err(|source| QualityError::Walk { source })?;
            scanned
                .findings
                .extend(check_text(&shown(repo, path), &text, limits));
        }
        scanned.files = scanned.files.saturating_add(sources.len());
        scanned
            .roots
            .push(json!({"root": shown(repo, &dir), "rust_files": count(sources.len())}));
    }
    Ok(scanned)
}
