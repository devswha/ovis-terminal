//! The replay record carries offsets as canonical decimal strings and nothing else.

use serde_json::json;

use super::{GraphicsReleaseReason, GraphicsReleased, GraphicsReplayedData};
use crate::schema::scalar::{ByteOffset, OpenEnum};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn offsets_round_trip_u64_max_as_decimal_strings() -> TestResult {
    let data = GraphicsReplayedData::new(
        vec![ByteOffset::new(u64::MAX)],
        vec![GraphicsReleased::new(
            ByteOffset::new(u64::MAX),
            GraphicsReleaseReason::StoreFull,
        )],
    );
    let value = serde_json::to_value(&data)?;
    assert_eq!(
        value,
        json!({
            "replayed": ["18446744073709551615"],
            "released": [{"output_offset": "18446744073709551615", "reason": "store_full"}],
        })
    );
    assert_eq!(serde_json::from_value::<GraphicsReplayedData>(value)?, data);
    Ok(())
}

#[test]
fn numeric_offsets_and_extra_fields_are_refused() {
    for refused in [
        json!({"replayed": [5], "released": []}),
        json!({"replayed": [], "released": [{"output_offset": 5, "reason": "erased"}]}),
        json!({"replayed": [], "released": [], "payload": "AAAA"}),
        json!({"replayed": [], "released": [{"output_offset": "5", "reason": "erased", "bytes": "1"}]}),
    ] {
        let decoded = serde_json::from_value::<GraphicsReplayedData>(refused.clone());
        assert!(decoded.is_err(), "{refused}");
    }
}

#[test]
fn a_release_reason_added_later_decodes_as_unknown() -> TestResult {
    let decoded: GraphicsReleased =
        serde_json::from_value(json!({"output_offset": "7", "reason": "evicted_by_policy"}))?;
    assert_eq!(decoded.output_offset, ByteOffset::new(7));
    assert_eq!(
        decoded.reason,
        OpenEnum::Unknown("evicted_by_policy".to_owned())
    );
    Ok(())
}
