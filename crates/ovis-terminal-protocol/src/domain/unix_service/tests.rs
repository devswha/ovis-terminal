//! Round trips, refusals and wire tokens of the discovery and status records.

use proptest::prelude::*;
use serde_json::json;

use super::{BootProof, DiscoveryRecord, EndedTerminal, Observed, Prerequisite, StaleReason};
use crate::schema::ids::{ProcessPid, ServiceLabel, StartToken, TerminalId};
use crate::schema::methods::service::ServiceManager;
use crate::schema::scalar::Incarnation;

fn record(pid: u32, token: String, address: String) -> DiscoveryRecord {
    let proof = BootProof::new(ProcessPid::new(pid), StartToken::new(token));
    DiscoveryRecord::new(
        ServiceLabel::new("ovis-terminal-holder-a".to_owned()),
        address,
        proof,
    )
    .with_manager(Some(ServiceManager::SystemdUser))
}

proptest! {
    #[test]
    fn a_record_round_trips_exactly(pid in any::<u32>(), token in ".{0,40}", address in "/[a-z/]{1,60}") {
        let written = record(pid, token, address);
        let bytes = serde_json::to_vec(&written).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(DiscoveryRecord::parse(&bytes).ok(), Some(written));
    }
}

#[test]
fn another_format_or_an_extra_field_is_refused_with_its_kind() {
    let mut value =
        serde_json::to_value(record(7, "t".to_owned(), "/s".to_owned())).unwrap_or_default();
    let kind = |value: &serde_json::Value| {
        DiscoveryRecord::parse(value.to_string().as_bytes())
            .map(drop)
            .map_err(|e| e.kind())
    };
    assert_eq!(kind(&value), Ok(()));
    value
        .as_object_mut()
        .unwrap()
        .insert("liveness".to_owned(), json!(true));
    assert_eq!(kind(&value), Err("descriptor_malformed"));
    value.as_object_mut().unwrap().insert(
        "format".to_owned(),
        json!("ovis-terminal-holder-endpoint/2"),
    );
    assert_eq!(kind(&value), Err("descriptor_unsupported"));
    assert_eq!(kind(&json!([])), Err("descriptor_unsupported"));
}

#[test]
fn prerequisite_tokens_are_their_serde_form() {
    for item in [
        Prerequisite::ServiceManager,
        Prerequisite::SystemdUserManager,
        Prerequisite::UserLinger,
        Prerequisite::LaunchdUserDomain,
    ] {
        assert_eq!(serde_json::to_value(item).ok(), Some(json!(item.token())));
    }
}

#[test]
fn the_state_tag_is_the_server_state_token() {
    let stale = Observed::StaleEndpoint {
        record: None,
        reason: StaleReason::ProcessGone,
    };
    for observed in [
        Observed::Stopped,
        Observed::Unreachable { record: None },
        stale,
    ] {
        let encoded = serde_json::to_value(&observed).unwrap_or_default();
        assert_eq!(
            encoded.get("state"),
            serde_json::to_value(observed.state()).ok().as_ref()
        );
        assert_eq!(
            serde_json::from_value::<Observed>(encoded).ok(),
            Some(observed)
        );
    }
}

#[test]
fn incarnations_are_exact_decimal_strings_and_numbers_are_refused() {
    let ended = EndedTerminal::new(TerminalId::new("t1".to_owned()), Incarnation::new(u64::MAX))
        .with_process(
            Incarnation::new(u64::MAX),
            BootProof::new(ProcessPid::new(9), StartToken::new("s".to_owned())),
        );
    let encoded = serde_json::to_value(&ended).unwrap_or_default();
    assert_eq!(
        encoded.get("terminal_incarnation"),
        Some(&json!("18446744073709551615"))
    );
    assert_eq!(
        serde_json::from_value::<EndedTerminal>(encoded.clone()).ok(),
        Some(ended)
    );
    let mut numeric = encoded;
    numeric
        .as_object_mut()
        .unwrap()
        .insert("terminal_incarnation".to_owned(), json!(7));
    assert!(serde_json::from_value::<EndedTerminal>(numeric).is_err());
}
