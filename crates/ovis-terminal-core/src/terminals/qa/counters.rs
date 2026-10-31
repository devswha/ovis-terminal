//! Canonical decimal identities at the terminal request boundary.

use ovis_terminal_core::registry::{
    ProcessGeneration, ProcessHandle, ProcessId, ProcessIdentity, ProcessPid, ProcessStartToken,
    TerminalId,
};
use ovis_terminal_core::terminals::KillRequest;
use proptest::prelude::*;

fn request(
    generation: u64,
    pid: u64,
) -> Result<KillRequest, ovis_terminal_core::registry::IdentityError> {
    Ok(KillRequest::immediate(
        TerminalId::try_from(1)?,
        ProcessIdentity::new(
            ProcessId::try_from(1)?,
            ProcessGeneration::new(generation)?,
            ProcessHandle::new(
                ProcessPid::try_from(pid)?,
                ProcessStartToken::new("kernel-token")?,
            ),
        ),
    ))
}

#[test]
fn maximum_counter_round_trip_rejects_numeric_json() {
    // Given: counters at the exact u64 boundary.
    let input = request(u64::MAX, u64::MAX).expect("nonzero counters");
    let bytes = serde_json::to_vec(&input).expect("encode");
    assert_eq!(KillRequest::decode(&bytes).expect("decode"), input);
    let mut wire: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON");
    assert_eq!(
        wire.pointer("/expected/generation")
            .and_then(serde_json::Value::as_str),
        Some("18446744073709551615")
    );

    // When: a sender substitutes a JSON number.
    *wire
        .pointer_mut("/expected/generation")
        .expect("generation field") = serde_json::json!(7);
    let error = KillRequest::decode(&serde_json::to_vec(&wire).expect("encode numeric input"))
        .expect_err("numeric generation rejected");

    // Then: the terminal boundary reports a typed invalid-parameters error with a source.
    assert_eq!(error.kind(), "invalid_params");
    assert!(std::error::Error::source(&error).is_some());
}

proptest! {
    #[test]
    fn process_identity_requests_round_trip(generation in 1..=u64::MAX, pid in 1..=u64::MAX) {
        let input = request(generation, pid).expect("nonzero counters");
        let bytes = serde_json::to_vec(&input).expect("encode");
        prop_assert_eq!(KillRequest::decode(&bytes).expect("decode"), input);
    }
}
