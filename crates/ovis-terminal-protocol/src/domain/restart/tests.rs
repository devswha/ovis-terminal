use proptest::prelude::*;
use serde_json::json;

use super::identity::{Cause, Field, fate, markdown};
use super::{Delivery, InterruptedOp, InterruptedOutcome, RestartPhase};
use crate::errors::OpId;
use crate::schema::ids::ServerBootId;
use crate::schema::methods::server::ServerTransitionResult;
use crate::schema::scalar::{ByteCount, ByteOffset, Incarnation};

fn outcome() -> impl Strategy<Value = InterruptedOutcome> {
    prop_oneof![
        Just(InterruptedOutcome::Committed {}),
        Just(InterruptedOutcome::NotStarted {}),
        Just(InterruptedOutcome::Failed {}),
        Just(InterruptedOutcome::Cancelled {}),
        Just(InterruptedOutcome::OutcomeUnknown {
            delivery: Delivery::Unproven {}
        }),
        (any::<u64>(), any::<u64>()).prop_map(|(bytes, offset)| {
            InterruptedOutcome::OutcomeUnknown {
                delivery: Delivery::Written {
                    written_bytes: ByteCount::new(bytes),
                    output_offset: ByteOffset::new(offset),
                },
            }
        }),
    ]
}

proptest! {
    #[test]
    fn an_interrupted_operation_round_trips_exactly(op in "[a-z0-9-]{1,36}", outcome in outcome()) {
        let written = InterruptedOp::new(OpId::new(op), outcome);
        let bytes = serde_json::to_vec(&written).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let read: InterruptedOp = serde_json::from_slice(&bytes).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(read, written);
    }
}

#[test]
fn a_written_offset_is_a_decimal_string_even_at_u64_max_and_a_number_is_refused() {
    let unknown = InterruptedOutcome::OutcomeUnknown {
        delivery: Delivery::Written {
            written_bytes: ByteCount::new(u64::MAX),
            output_offset: ByteOffset::new(u64::MAX),
        },
    };
    let value = serde_json::to_value(unknown).unwrap_or_default();
    assert_eq!(
        value,
        json!({"outcome": "outcome_unknown", "delivery": {
            "state": "written",
            "written_bytes": "18446744073709551615",
            "output_offset": "18446744073709551615"
        }})
    );
    let numeric = json!({"outcome": "outcome_unknown", "delivery": {"state": "written", "written_bytes": 7, "output_offset": "7"}});
    assert!(serde_json::from_value::<InterruptedOutcome>(numeric).is_err());
    let committed = json!({"outcome": "committed"});
    assert_eq!(
        serde_json::from_value::<InterruptedOutcome>(committed).ok(),
        Some(InterruptedOutcome::Committed {})
    );
    let extra = json!({"outcome": "committed", "retry": true});
    assert!(serde_json::from_value::<InterruptedOutcome>(extra).is_err());
    let unproven = json!({"outcome": "outcome_unknown", "delivery": {"state": "unproven", "written_bytes": "1"}});
    assert!(serde_json::from_value::<InterruptedOutcome>(unproven).is_err());
}

#[test]
fn only_an_unknown_outcome_is_uncertain() {
    let uncertain = |outcome| InterruptedOp::new(OpId::new("op".to_owned()), outcome).uncertain();
    assert!(uncertain(InterruptedOutcome::OutcomeUnknown {
        delivery: Delivery::Unproven {}
    }));
    for settled in [
        InterruptedOutcome::Committed {},
        InterruptedOutcome::NotStarted {},
        InterruptedOutcome::Failed {},
        InterruptedOutcome::Cancelled {},
    ] {
        assert!(!uncertain(settled));
    }
}

#[test]
fn the_phases_form_one_cycle_and_only_pre_handover_phases_can_roll_back() {
    let mut phase = RestartPhase::Running;
    let mut visited = vec![phase];
    loop {
        phase = phase.next();
        if phase == RestartPhase::Running {
            break;
        }
        assert!(!visited.contains(&phase), "{phase:?} repeats");
        visited.push(phase);
    }
    assert_eq!(visited.len(), 7);
    let rollback: Vec<_> = visited.iter().filter(|p| p.before_handover()).collect();
    assert_eq!(
        rollback,
        [
            &RestartPhase::Running,
            &RestartPhase::IntentPersisted,
            &RestartPhase::Suspended,
            &RestartPhase::Announced
        ]
    );
}

#[test]
fn every_table_token_is_its_serde_form() {
    for cause in Cause::ALL {
        assert_eq!(serde_json::to_value(cause).ok(), Some(json!(cause.token())));
        for field in Field::ALL {
            let fate = fate(cause, field);
            assert_eq!(serde_json::to_value(fate).ok(), Some(json!(fate.token())));
        }
    }
    for field in Field::ALL {
        assert_eq!(serde_json::to_value(field).ok(), Some(json!(field.token())));
    }
}

#[test]
fn the_published_document_carries_the_table_verbatim() {
    let document = include_str!("../../../../../docs/identity-transitions.md");
    assert!(
        document.contains(&markdown()),
        "regenerate the table in docs/identity-transitions.md from identity::markdown()"
    );
}

#[test]
fn a_restart_result_carries_both_boots_and_decimal_incarnations() {
    let result = ServerTransitionResult::restarted(
        (
            ServerBootId::new("boot:1".to_owned()),
            ServerBootId::new("boot:2".to_owned()),
        ),
        vec![Incarnation::new(u64::MAX)],
        vec![OpId::new("op-a".to_owned())],
    );
    assert_eq!(
        serde_json::to_value(result).unwrap_or_default(),
        json!({
            "server_boot_id_old": "boot:1",
            "server_boot_id_new": "boot:2",
            "preserved_incarnations": ["18446744073709551615"],
            "ended_incarnations": [],
            "interrupted_ops": ["op-a"],
        })
    );
    let stopped = ServerTransitionResult::stopped(
        ServerBootId::new("boot:1".to_owned()),
        vec![Incarnation::new(1)],
        vec![Incarnation::new(2)],
    );
    let stopped = serde_json::to_value(stopped).unwrap_or_default();
    assert_eq!(stopped.get("server_boot_id_new"), Some(&json!(null)));
    assert_eq!(stopped.get("ended_incarnations"), Some(&json!(["2"])));
}
