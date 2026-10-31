//! Lease epochs and events on the wire: canonical decimal strings only, exact round trips.

use ovis_terminal_core::leases::{ControlSlot, LeaseEvent, Takeover};
use ovis_terminal_core::registry::LeaseId;
use ovis_terminal_protocol::schema::methods::attach::TakeoverRequest;
use proptest::prelude::{ProptestConfig, prop_assert_eq, proptest};
use serde_json::{Value, json};

use super::TestResult;

fn request(epoch: &Value) -> Result<TakeoverRequest, serde_json::Error> {
    serde_json::from_value(json!({ "lease_epoch": epoch, "reason": "wire" }))
}

fn revoked(epoch: &Value, lease_id: &str) -> Value {
    json!({
        "revoked": {
            "lease": {
                "lease_id": lease_id,
                "slot": "interact",
                "epoch": epoch,
                "owner": { "principal": "operator", "client": "tui" },
            },
            "reason": "taken_over",
            "displaced_by": null,
        }
    })
}

#[test]
fn takeover_epochs_are_canonical_nonzero_decimal_strings() -> TestResult {
    // Given wire takeover requests with numeric, padded, zero and maximal epochs.
    let fresh = LeaseId::try_from(1)?;
    let numeric = request(&json!(3));
    let padded = request(&json!("03"));
    let zero = Takeover::new(ControlSlot::Interact, &request(&json!("0"))?, fresh).err();

    // When the maximal epoch is accepted.
    let max = Takeover::new(
        ControlSlot::Interact,
        &request(&json!(u64::MAX.to_string()))?,
        fresh,
    )?;

    // Then only the canonical decimal string reaches the lease table, exactly.
    assert!(numeric.is_err(), "numeric epoch accepted");
    assert!(padded.is_err(), "non-canonical epoch accepted");
    assert_eq!(zero.map(|error| error.kind()), Some("invalid_params"));
    assert_eq!(max.expected().get(), u64::MAX);
    Ok(())
}

#[test]
fn lease_events_round_trip_the_maximal_epoch_and_reject_numeric_counters() -> TestResult {
    // Given a revoked event at epoch u64::MAX and the same event with a numeric epoch.
    let encoded = revoked(&json!(u64::MAX.to_string()), "lease:18446744073709551615");
    let numeric = revoked(&json!(7), "lease:1");

    // When both are decoded.
    let decoded: LeaseEvent = serde_json::from_value(encoded.clone())?;
    let refused = serde_json::from_value::<LeaseEvent>(numeric);

    // Then the string form survives exactly and the numeric form is refused.
    assert_eq!(serde_json::to_value(&decoded)?, encoded);
    assert!(refused.is_err(), "numeric epoch decoded");
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn every_nonzero_epoch_and_lease_id_round_trips(epoch in 1_u64.., id in 1_u64..) {
        let encoded = revoked(&json!(epoch.to_string()), &format!("lease:{id}"));
        let decoded: LeaseEvent = serde_json::from_value(encoded.clone())
            .map_err(|error| proptest::test_runner::TestCaseError::fail(error.to_string()))?;
        let again = serde_json::to_value(&decoded)
            .map_err(|error| proptest::test_runner::TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(again, encoded);
    }
}
