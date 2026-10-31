use proptest::prelude::*;
use serde_json::{Value, json};

use super::{PluginContext, Unavailable};
use crate::errors::OpId;
use crate::schema::ids::{PluginActionId, PluginId, PluginRunId, TerminalId};
use crate::schema::methods::plugin::{
    PluginAction, PluginListResult, PluginRecord, PluginRunRecord, PluginRunState,
};
use crate::schema::scalar::ByteOffset;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn at(value: &Value, pointer: &str) -> Value {
    value.pointer(pointer).cloned().unwrap_or(Value::Null)
}

#[test]
fn a_settled_run_round_trips_with_a_decimal_floor_at_u64_max() -> TestResult {
    let record = PluginRunRecord::running(PluginRunId::new("plugin-run:op".to_owned()))
        .with_output("T0s=".to_owned(), ByteOffset::new(u64::MAX))
        .settled(PluginRunState::Failed, Some(7))
        .with_created(vec![TerminalId::new("t1".to_owned())]);

    let encoded = serde_json::to_value(&record)?;

    assert_eq!(at(&encoded, "/output_floor"), json!(u64::MAX.to_string()));
    assert_eq!(at(&encoded, "/truncated"), json!(true));
    assert_eq!(at(&encoded, "/state"), json!("failed"));
    assert_eq!(at(&encoded, "/exit_code"), json!(7));
    assert_eq!(serde_json::from_value::<PluginRunRecord>(encoded)?, record);
    Ok(())
}

#[test]
fn a_numeric_output_floor_is_rejected() -> TestResult {
    let mut encoded = serde_json::to_value(PluginRunRecord::running(PluginRunId::new(
        "plugin-run:op".to_owned(),
    )))?;
    *encoded.pointer_mut("/output_floor").ok_or("output_floor")? = json!(3);

    assert!(serde_json::from_value::<PluginRunRecord>(encoded).is_err());
    Ok(())
}

#[test]
fn an_untruncated_tail_starts_at_offset_zero() -> TestResult {
    let record = PluginRunRecord::running(PluginRunId::new("plugin-run:op".to_owned()))
        .with_output("T0s=".to_owned(), ByteOffset::new(0));

    let encoded = serde_json::to_value(record)?;

    assert_eq!(at(&encoded, "/truncated"), json!(false));
    assert_eq!(at(&encoded, "/output_floor"), json!("0"));
    assert_eq!(at(&encoded, "/state"), json!("running"));
    assert_eq!(at(&encoded, "/exit_code"), Value::Null);
    Ok(())
}

#[test]
fn an_unavailable_action_lists_its_reason_token_and_contexts() -> TestResult {
    let action = PluginAction::new(
        PluginActionId::new("admin-action".to_owned()),
        json!({"type": "object"}),
        &[PluginContext::Machine],
    )
    .unavailable_because(Unavailable::PluginDisabled);
    let listing = PluginListResult::new(vec![
        PluginRecord::new(
            PluginId::new("fixture".to_owned()),
            "1.0.0".to_owned(),
            false,
        )
        .with_actions(vec![action]),
    ]);

    let encoded = serde_json::to_value(listing)?;

    assert_eq!(
        at(&encoded, "/plugins/0/actions/0/unavailable"),
        json!("plugin_disabled")
    );
    assert_eq!(
        at(&encoded, "/plugins/0/actions/0/contexts"),
        json!(["machine"])
    );
    assert_eq!(at(&encoded, "/plugins/0/enabled"), json!(false));
    Ok(())
}

#[test]
fn every_unavailability_reason_has_a_distinct_token() {
    let tokens = [
        Unavailable::PluginDisabled,
        Unavailable::PlatformUnsupported,
        Unavailable::ExecutableMissing,
        Unavailable::DefinitionInvalid,
    ]
    .map(Unavailable::as_str);
    let mut unique = tokens.to_vec();
    unique.sort_unstable();
    unique.dedup();

    assert_eq!(unique.len(), tokens.len());
}

#[test]
fn a_foreign_run_identity_names_no_operation() {
    assert_eq!(PluginRunId::new("run-7".to_owned()).operation(), None);
    assert_eq!(PluginRunId::new("plugin-run:".to_owned()).operation(), None);
}

proptest! {
    #[test]
    fn every_operation_maps_to_one_run_and_back(op in "[0-9a-f-]{1,40}") {
        let op = OpId::new(op);
        let run = PluginRunId::of_operation(&op);

        prop_assert_eq!(run.operation(), Some(op));
    }
}
