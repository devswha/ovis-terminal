//! Observation classification under arbitrary content.

use std::path::Path;

use ovis_terminal_protocol::domain::session_binding::HistoryChange;
use proptest::prelude::*;

use crate::conversation::binding::observe::read;
use crate::conversation::binding::{SourceRef, Stores};

fn transcript(
    dir: &Path,
    name: &str,
) -> Result<crate::conversation::binding::TranscriptPath, TestCaseError> {
    let stores =
        Stores::new(&[("s", dir.to_path_buf())]).map_err(|e| TestCaseError::fail(e.to_string()))?;
    let session = serde_json::from_value(serde_json::json!({
        "source": "s", "kind": "path", "value": dir.join(name).to_string_lossy()
    }))
    .map_err(|e| TestCaseError::fail(e.to_string()))?;
    match stores
        .source(&session)
        .map_err(|e| TestCaseError::fail(e.to_string()))?
    {
        SourceRef::Transcript(path) => Ok(path),
        other => Err(TestCaseError::fail(format!("{other:?}"))),
    }
}

proptest! {
    #[test]
    fn appends_rewrites_and_truncations_are_told_apart(
        base in proptest::collection::vec(any::<u8>(), 1..256),
        suffix in proptest::collection::vec(any::<u8>(), 1..64),
        flip in any::<prop::sample::Index>(),
    ) {
        let dir = std::env::temp_dir().join(format!("ovis-t36-observe-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let file = dir.join("t.jsonl");
        std::fs::write(&file, &base).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let path = transcript(&dir, "t.jsonl")?;
        let recorded = read(&path).map_err(|e| TestCaseError::fail(e.to_string()))?.observation;
        let classify = |bytes: &[u8]| -> Result<Option<HistoryChange>, TestCaseError> {
            std::fs::write(&file, bytes).map_err(|e| TestCaseError::fail(e.to_string()))?;
            Ok(read(&path).map_err(|e| TestCaseError::fail(e.to_string()))?.change_since(&recorded))
        };
        prop_assert_eq!(classify(&base)?, None);
        prop_assert_eq!(classify(&[base.as_slice(), &suffix].concat())?, Some(HistoryChange::Append));
        let mut rewritten = base.clone();
        let at = flip.index(rewritten.len());
        if let Some(byte) = rewritten.get_mut(at) {
            *byte = byte.wrapping_add(1);
        }
        prop_assert_eq!(classify(&rewritten)?, Some(HistoryChange::Rewrite));
        prop_assert_eq!(classify(base.split_last().map(|(_, rest)| rest).unwrap_or_default())?, Some(HistoryChange::Truncate));
        std::fs::remove_dir_all(&dir).map_err(|e| TestCaseError::fail(e.to_string()))?;
    }
}
