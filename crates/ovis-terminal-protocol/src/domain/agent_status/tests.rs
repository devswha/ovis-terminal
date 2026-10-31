use proptest::prelude::*;
use serde_json::json;

use super::{
    DecodeFailure, IngestCounters, OscDecodeError, OscFields, OscProvenance, OscTextField,
};
use crate::errors::{BoundaryError, ErrorData, ErrorKind};
use crate::schema::methods::status::StatusReportParams;
use crate::schema::scalar::ReportSeq;

#[test]
fn a_stale_rejection_carries_current_seq_as_an_exact_decimal_string()
-> Result<(), serde_json::Error> {
    let error = BoundaryError::domain(ErrorKind::from("stale_seq".to_owned()), false)
        .with_current_seq(ReportSeq::new(u64::MAX));
    let wire = serde_json::to_value(error.data())?;
    assert_eq!(
        wire.get("current_seq"),
        Some(&json!("18446744073709551615"))
    );
    let back: ErrorData = serde_json::from_value(wire)?;
    assert_eq!(back.current_seq, Some(ReportSeq::new(u64::MAX)));
    assert!(
        serde_json::from_value::<ErrorData>(json!({
            "kind": "stale_seq", "hint": "h", "retryable": false, "current_seq": 4
        }))
        .is_err()
    );
    Ok(())
}

#[test]
fn a_numeric_report_sequence_is_refused_at_params_seq() {
    let params = |seq| {
        json!({"op_id": "o", "target": {"name": "agent"}, "process_token": "t",
               "record": {"state": "done"}, "seq": seq})
    };
    assert!(serde_json::from_value::<StatusReportParams>(params(json!("3"))).is_ok());
    let refused = serde_json::from_value::<StatusReportParams>(params(json!(3)));
    assert!(refused.is_err());
}

proptest! {
    #[test]
    fn provenance_and_counters_round_trip(
        raw in ".{0,64}", state in "[a-z]{1,10}", msg in proptest::option::of("[ -~]{0,32}"),
        oversize in any::<u64>(), malformed in any::<u64>(),
    ) {
        let fields = OscFields { state, msg_b64: msg, ..OscFields::default() };
        let mut provenance = OscProvenance::new(raw, fields);
        provenance.decode_errors.push(OscDecodeError::new(OscTextField::Msg, DecodeFailure::InvalidBase64));
        let text = serde_json::to_string(&provenance).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let back: OscProvenance = serde_json::from_str(&text).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(back, provenance);
        let counters = IngestCounters::new(oversize, malformed, oversize ^ malformed);
        let wire = serde_json::to_value(counters).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(wire.get("oversize"), Some(&json!(oversize.to_string())));
        let back: IngestCounters = serde_json::from_value(wire).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(back, counters);
    }
}
