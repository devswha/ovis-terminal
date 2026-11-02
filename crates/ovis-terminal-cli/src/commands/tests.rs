use ovis_terminal_protocol::schema::methods::status::StatusReportParams;
use serde_json::json;

use super::{CommandError, Planned, Supplied, TargetUse, VerbSpec, check, plan};

const REPORT: VerbSpec = VerbSpec {
    method: "status.report",
    check: check::<StatusReportParams>,
    token: true,
    target: TargetUse::Required,
};

fn supplied(target: Option<serde_json::Value>, params: &str) -> Supplied {
    Supplied {
        target,
        op_id: Some("op-1".to_owned()),
        params: params.to_owned(),
    }
}

#[test]
fn flags_and_the_process_token_fill_the_params_object() {
    let planned = plan(
        REPORT,
        supplied(Some(json!({"address": "t1"})), r#"{"seq":"1"}"#),
        Some("tok"),
    );
    let Ok(Planned::Send { params, .. }) = planned else {
        panic!("expected a send plan");
    };
    assert_eq!(
        params,
        json!({"seq": "1", "target": {"address": "t1"}, "op_id": "op-1", "process_token": "tok"})
    );
}

#[test]
fn an_explicit_token_in_params_wins_over_the_inherited_one() {
    let planned = plan(
        REPORT,
        supplied(
            Some(json!({"address": "t1"})),
            r#"{"process_token":"mine"}"#,
        ),
        Some("inherited"),
    );
    let Ok(Planned::Send { params, .. }) = planned else {
        panic!("expected a send plan");
    };
    assert_eq!(params.get("process_token"), Some(&json!("mine")));
}

#[test]
fn a_missing_required_target_plans_needs_target_with_every_supplied_field_to_check() {
    let planned = plan(REPORT, supplied(None, r#"{"seq":"1"}"#), Some("tok"));
    let Ok(Planned::NeedsTarget { probe, .. }) = planned else {
        panic!("expected a needs-target plan");
    };
    assert_eq!(
        probe,
        json!({"seq": "1", "target": {"address": super::STAND_IN_ADDRESS}, "op_id": "op-1", "process_token": "tok"})
    );
}

#[test]
fn a_field_named_twice_or_non_object_params_are_refused() {
    let twice = plan(
        REPORT,
        supplied(
            Some(json!({"address": "t1"})),
            r#"{"target":{"address":"t2"}}"#,
        ),
        None,
    );
    assert!(matches!(
        twice,
        Err(CommandError::Repeated { field: "target" })
    ));
    let listed = plan(REPORT, supplied(None, "[1]"), None);
    assert_eq!(
        listed.map(drop).map_err(|error| error.kind()),
        Err("invalid_params")
    );
}
