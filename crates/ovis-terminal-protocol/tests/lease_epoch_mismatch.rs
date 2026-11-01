//! A stale-epoch refusal names the lease that holds the slot now, with its epoch as an
//! exact decimal string.

use ovis_terminal_protocol::errors::{BoundaryError, ErrorData};
use ovis_terminal_protocol::schema::ids::{ClientId, LeaseId};
use ovis_terminal_protocol::schema::records::{LeaseRecord, LeaseRole};
use ovis_terminal_protocol::schema::scalar::LeaseEpoch;
use proptest::prelude::*;
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn refusal(epoch: u64) -> BoundaryError {
    let current = LeaseRecord::new(
        LeaseId::new("lease:1".to_owned()),
        LeaseEpoch::new(epoch),
        (LeaseRole::Interact, ClientId::new("cli".to_owned())),
    );
    BoundaryError::domain("lease_epoch_mismatch".to_owned().into(), false)
        .with_current_lease(current)
}

proptest! {
    #[test]
    fn the_current_lease_round_trips_every_epoch(epoch in any::<u64>()) {
        let wire = serde_json::to_value(refusal(epoch).data())
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let decoded: ErrorData = serde_json::from_value(wire.clone())
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(
            wire.get("current"),
            Some(&json!({"lease_id": "lease:1", "epoch": epoch.to_string(), "role": "interact", "holder": "cli"}))
        );
        prop_assert_eq!(decoded.current.map(|lease| lease.epoch.get()), Some(epoch));
    }
}

#[test]
fn the_maximum_epoch_survives_and_a_numeric_epoch_is_refused() -> TestResult {
    let wire = serde_json::to_value(refusal(u64::MAX).data())?;
    let decoded: ErrorData = serde_json::from_value(wire.clone())?;
    assert_eq!(
        decoded.current.map(|lease| lease.epoch),
        Some(LeaseEpoch::new(u64::MAX))
    );
    let mut numeric = wire;
    *numeric.pointer_mut("/current/epoch").ok_or("epoch")? = json!(u64::MAX);
    assert!(serde_json::from_value::<ErrorData>(numeric).is_err());
    Ok(())
}

#[test]
fn a_refusal_without_a_current_lease_omits_the_field() -> TestResult {
    let refused = BoundaryError::domain("lease_conflict".to_owned().into(), false);
    let wire = serde_json::to_value(refused.data())?;
    assert_eq!(wire.get("current"), None);
    Ok(())
}
