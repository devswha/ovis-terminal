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

/// A transcript bound inside the store, then swapped for a link that leaves it, is refused
/// at read time; the outside file's bytes are never returned.
#[cfg(unix)]
#[test]
fn a_transcript_replaced_by_an_outside_link_after_binding_is_refused() -> Result<(), TestCaseError>
{
    use crate::conversation::binding::BindingError;
    let fail = |e: std::io::Error| TestCaseError::fail(e.to_string());
    let base = std::env::temp_dir().join(format!("ovis-t64-link-{}", uuid::Uuid::now_v7()));
    let (store, outside) = (base.join("store"), base.join("outside"));
    std::fs::create_dir_all(store.join("nested")).map_err(fail)?;
    std::fs::create_dir_all(&outside).map_err(fail)?;
    std::fs::write(store.join("t.jsonl"), b"inside\n").map_err(fail)?;
    std::fs::write(store.join("nested/n.jsonl"), b"inside\n").map_err(fail)?;
    std::fs::write(outside.join("t.jsonl"), b"outside secret\n").map_err(fail)?;
    std::fs::write(outside.join("n.jsonl"), b"outside secret\n").map_err(fail)?;
    let file = transcript(&store, "t.jsonl")?;
    let nested = transcript(&store, "nested/n.jsonl")?;
    prop_assert_eq!(
        read(&file)
            .map_err(|e| TestCaseError::fail(e.to_string()))?
            .bytes,
        b"inside\n"
    );

    std::fs::remove_file(store.join("t.jsonl")).map_err(fail)?;
    std::os::unix::fs::symlink(outside.join("t.jsonl"), store.join("t.jsonl")).map_err(fail)?;
    std::fs::remove_dir_all(store.join("nested")).map_err(fail)?;
    std::os::unix::fs::symlink(&outside, store.join("nested")).map_err(fail)?;

    for path in [&file, &nested] {
        let refused = read(path);
        prop_assert!(
            matches!(refused, Err(BindingError::OutsideStores)),
            "{refused:?}"
        );
    }
    std::fs::remove_dir_all(&base).map_err(fail)?;
    Ok(())
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
