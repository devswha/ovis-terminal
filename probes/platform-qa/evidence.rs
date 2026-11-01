//! Appending one record to an evidence file the conformance runner may also write.

use std::path::Path;

use serde_json::Value;

use super::error::QaError;

/// Adds `record` to the evidence at `path` the way `conformance --append` does: a
/// missing file becomes the record, an object becomes `[object, record]` and an
/// array gains the record at its end.
///
/// # Errors
///
/// The existing file is unreadable or not JSON, or the file cannot be written.
pub fn append(path: &Path, record: Value) -> Result<(), QaError> {
    let io = |operation| move |source| QaError::Io { operation, source };
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(io("create the evidence directory"))?;
    }
    let document = match std::fs::read(path) {
        Ok(bytes) => {
            let earlier: Value =
                serde_json::from_slice(&bytes).map_err(|source| QaError::Json {
                    what: path.display().to_string(),
                    source,
                })?;
            match earlier {
                Value::Array(mut runs) => {
                    runs.push(record);
                    Value::Array(runs)
                }
                single => Value::Array(vec![single, record]),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => record,
        Err(error) => return Err(io("read the evidence file")(error)),
    };
    let bytes = serde_json::to_vec_pretty(&document).map_err(|source| QaError::Json {
        what: "the evidence document".to_owned(),
        source,
    })?;
    std::fs::write(path, bytes).map_err(io("write the evidence file"))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::append;

    #[test]
    fn records_accumulate_after_an_earlier_single_run() {
        let dir = std::env::temp_dir().join(format!("ovis-qa-evidence-{}", std::process::id()));
        let path = dir.join("nested").join("evidence.json");
        drop(std::fs::remove_dir_all(&dir));
        append(&path, json!({"run": 1})).unwrap();
        let first: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(first, json!({"run": 1}));
        append(&path, json!({"run": 2})).unwrap();
        append(&path, json!({"run": 3})).unwrap();
        let all: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(all, json!([{"run": 1}, {"run": 2}, {"run": 3}]));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_corrupt_evidence_file_is_an_error_and_left_untouched() {
        let dir = std::env::temp_dir().join(format!("ovis-qa-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("evidence.json");
        std::fs::write(&path, b"{not json").unwrap();
        let refused = append(&path, json!({"run": 1})).unwrap_err();
        assert_eq!(refused.kind(), "invalid_json");
        assert_eq!(std::fs::read(&path).unwrap(), b"{not json");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
