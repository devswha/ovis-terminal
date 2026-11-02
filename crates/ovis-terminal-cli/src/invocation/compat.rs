//! `schema compat --released <path>`: this build's OpenRPC document against a released
//! baseline, offline. Same-generation rules: inputs only widen, outputs only grow.

use std::fs::File;
use std::io::{self, Read as _};
use std::path::Path;

use ovis_terminal_protocol::schema::{SchemaError, breaking_changes, render_openrpc};
use serde_json::{Value, json};

use super::args::SchemaPart;
use super::error::CliError;
use super::plan::Plan;

/// Largest baseline read; the released document is about 0.5 MiB.
const MAX_BASELINE_BYTES: u64 = 64 * 1024 * 1024;

fn read(path: &Path) -> Result<Value, CliError> {
    let unreadable = |source| CliError::BaselineUnreadable {
        path: path.to_path_buf(),
        source,
    };
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(unreadable)?
        .take(MAX_BASELINE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(unreadable)?;
    if u64::try_from(bytes.len()).map_or(true, |len| len > MAX_BASELINE_BYTES) {
        let bound = io::Error::new(
            io::ErrorKind::FileTooLarge,
            "over the 64 MiB baseline bound",
        );
        return Err(unreadable(bound));
    }
    serde_json::from_slice(&bytes).map_err(|source| CliError::BaselineJson {
        path: path.to_path_buf(),
        source,
    })
}

/// `schema [<part>] [--released <path>]`: `--released` goes with `compat` and only with it.
pub(super) fn plan(
    part: SchemaPart,
    released: Option<std::path::PathBuf>,
) -> Result<Plan, CliError> {
    match (part, released) {
        (SchemaPart::Compat, Some(path)) => Ok(Plan::Compat(path)),
        (SchemaPart::Compat, None) | (_, Some(_)) => Err(CliError::ReleasedFlag),
        (part, None) => Ok(Plan::Schema(part)),
    }
}

/// `info.x-generation` as its decimal text, or a malformed-baseline error.
fn generation(document: &Value, path: &Path) -> Result<String, CliError> {
    match document.pointer("/info/x-generation") {
        Some(Value::String(text)) => Ok(text.clone()),
        Some(Value::Number(number)) => Ok(number.to_string()),
        _ => Err(CliError::BaselineSchema {
            path: path.to_path_buf(),
            source: SchemaError::MalformedBaseline {
                location: "info.x-generation".to_owned(),
            },
        }),
    }
}

/// The verdict for `path`; any break, or another generation, is a typed failure.
pub(super) fn compat(path: &Path) -> Result<Value, CliError> {
    let released = read(path)?;
    let rendered = render_openrpc().map_err(CliError::Schema)?;
    let current: Value = serde_json::from_str(&rendered).map_err(CliError::Encode)?;
    let (was, now) = (generation(&released, path)?, generation(&current, path)?);
    if was != now {
        return Err(CliError::GenerationMismatch {
            released: was,
            current: now,
        });
    }
    let breaks =
        breaking_changes(&released, &current).map_err(|source| CliError::BaselineSchema {
            path: path.to_path_buf(),
            source,
        })?;
    if !breaks.is_empty() {
        return Err(CliError::CompatibilityBreak { breaks });
    }
    Ok(json!({
        "compatible": true,
        "released": path,
        "released_version": released.pointer("/info/version"),
        "version": crate::VERSION,
        "generation": now,
    }))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ovis_terminal_protocol::schema::render_openrpc;
    use serde_json::{Value, json};

    use super::compat;
    use crate::invocation::error::CliError;

    fn current() -> Value {
        render_openrpc()
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null)
    }

    fn written(document: &Value, name: &str) -> std::path::PathBuf {
        let dir = std::env::var_os("OVIS_FIXTURE_ROOT")
            .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
            .join(format!("ovt-compat-{}-{name}", std::process::id()));
        let path = dir.join("released.openrpc.json");
        let wrote = std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(&path, document.to_string()));
        assert!(wrote.is_ok(), "{wrote:?}");
        path
    }

    #[test]
    fn this_build_is_compatible_with_its_own_document() {
        let path = written(&current(), "self");
        let verdict = compat(&path).map_err(|error| error.kind());
        assert_eq!(
            verdict.map(|value| value.get("compatible").cloned()),
            Ok(Some(json!(true)))
        );
    }

    #[test]
    fn a_baseline_requiring_an_output_this_build_lacks_is_a_compatibility_break() {
        let mut released = current();
        let required = released
            .pointer_mut("/components/schemas/ServerStatusResult/required")
            .and_then(Value::as_array_mut);
        assert!(required.is_some(), "ServerStatusResult has a required list");
        if let Some(list) = required {
            list.push(json!("released_only"));
        }
        if let Some(properties) = released
            .pointer_mut("/components/schemas/ServerStatusResult/properties")
            .and_then(Value::as_object_mut)
        {
            properties.insert("released_only".to_owned(), json!({"type": "string"}));
        }
        let error = compat(&written(&released, "break"));
        let Err(CliError::CompatibilityBreak { breaks }) = error else {
            panic!("expected a compatibility break, got {error:?}");
        };
        assert!(
            breaks
                .iter()
                .any(|change| change.path.contains("released_only")),
            "{breaks:?}"
        );
    }

    #[test]
    fn another_generation_is_refused_before_any_comparison() {
        let mut released = current();
        if let Some(info) = released.pointer_mut("/info").and_then(Value::as_object_mut) {
            info.insert("x-generation".to_owned(), json!("2"));
        }
        let error = compat(&written(&released, "generation")).map_err(|error| error.kind());
        assert_eq!(error.map(|_| ()), Err("generation_mismatch"));
    }

    #[test]
    fn a_missing_or_non_json_baseline_has_its_own_kind() {
        let missing = compat(Path::new("/nonexistent/released.openrpc.json"));
        assert_eq!(
            missing.map(|_| ()).map_err(|e| e.kind()),
            Err("baseline_unreadable")
        );
        let path = written(&json!(null), "null");
        let wrote = std::fs::write(&path, b"not json");
        assert!(wrote.is_ok());
        assert_eq!(
            compat(&path).map(|_| ()).map_err(|e| e.kind()),
            Err("baseline_malformed")
        );
    }
}
