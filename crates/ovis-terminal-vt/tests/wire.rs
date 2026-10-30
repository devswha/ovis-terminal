//! u64 offsets and revisions cross JSON as canonical decimal strings; numeric
//! or non-canonical forms are rejected.

use ovis_terminal_vt::{
    Checkpoint, ContentRevision, Geometry, HistoryLimit, ModeRevision, OutputOffset, Vt, VtConfig,
};
use proptest::prelude::*;

#[test]
fn u64_max_round_trips_exactly_as_a_string() {
    let encoded = serde_json::to_string(&OutputOffset::new(u64::MAX)).expect("encodes");
    assert_eq!(encoded, "\"18446744073709551615\"");
    let decoded: OutputOffset = serde_json::from_str(&encoded).expect("decodes");
    assert_eq!(decoded.get(), u64::MAX);
}

#[test]
fn numeric_and_non_canonical_forms_are_rejected() {
    for input in [
        "18446744073709551615",
        "3",
        "\"03\"",
        "\"-1\"",
        "\"+1\"",
        "\"\"",
        "\" 1\"",
        "\"18446744073709551616\"",
    ] {
        assert!(
            serde_json::from_str::<OutputOffset>(input).is_err(),
            "{input} must be rejected"
        );
        assert!(
            serde_json::from_str::<ContentRevision>(input).is_err(),
            "{input} must be rejected"
        );
        assert!(
            serde_json::from_str::<ModeRevision>(input).is_err(),
            "{input} must be rejected"
        );
    }
    assert_eq!(
        serde_json::from_str::<OutputOffset>("\"0\"")
            .expect("zero is canonical")
            .get(),
        0
    );
}

#[test]
fn checkpoint_with_a_numeric_offset_is_malformed() {
    let geometry = Geometry::new(10, 3).expect("valid geometry");
    let mut vt = Vt::new(VtConfig::new(geometry, HistoryLimit::new(5)));
    vt.feed(b"abc");
    let mut json: serde_json::Value =
        serde_json::from_slice(&vt.export_checkpoint().to_bytes().expect("encodes")).expect("json");
    assert_eq!(json.get("offset"), Some(&serde_json::json!("3")));
    assert_eq!(json.get("revision"), Some(&serde_json::json!("3")));
    *json.pointer_mut("/offset").expect("offset field") = serde_json::json!(3);
    let bytes = serde_json::to_vec(&json).expect("encodes");
    let error = Checkpoint::from_bytes(&bytes).expect_err("numeric offset rejected");
    assert_eq!(error.kind(), "checkpoint_malformed");
}

proptest! {
    #[test]
    fn every_u64_round_trips(value in any::<u64>()) {
        let offset = OutputOffset::new(value);
        let encoded = serde_json::to_string(&offset).expect("encodes");
        prop_assert_eq!(&encoded, &format!("\"{value}\""));
        prop_assert_eq!(serde_json::from_str::<OutputOffset>(&encoded).expect("decodes"), offset);
    }
}
