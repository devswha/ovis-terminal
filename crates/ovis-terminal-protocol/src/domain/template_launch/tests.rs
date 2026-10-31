use proptest::prelude::*;
use serde_json::{Value, json};

use super::{
    FirstMessageReason, FirstMessageRecord, FirstMessageState, LaunchSource, LaunchedTerminal,
    ReadinessContract, TemplateLaunchError, started_process,
};
use crate::errors::OpId;
use crate::rpc::parse_request;
use crate::schema::decode_params;
use crate::schema::ids::{MachineId, TerminalAddress, TerminalId};
use crate::schema::methods::terminal::{SpawnState, TerminalRunParams};
use crate::schema::records::AgentState;
use crate::schema::scalar::{EventSeq, Incarnation, OpenEnum, TimestampMs};

fn run_params(params: &Value) -> Result<TerminalRunParams, String> {
    let request = json!({"jsonrpc": "2.0", "id": "r", "method": "terminal.run", "params": params});
    let request = parse_request(&request.to_string()).map_err(|error| error.to_string())?;
    decode_params(&request).map_err(|error| error.field_path().to_owned())
}

fn base() -> Value {
    json!({"op_id": "0191f1a2-0000-7000-8000-000000000035", "geometry": {"cols": 80, "rows": 24}})
}

fn with(extra: &Value) -> Value {
    let mut params = base();
    if let (Some(target), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
        target.extend(extra.clone());
    }
    params
}

#[test]
fn a_top_level_template_without_deadline_selects_that_template_and_version() {
    let params = run_params(&with(&json!({
        "template": {"name": "build", "version": "missing"},
        "first_message": {"text": "x"}
    })))
    .expect("the QA launch shape decodes");
    let source = LaunchSource::of(&params).expect("one program");
    let LaunchSource::Template {
        name,
        version,
        task,
    } = source
    else {
        panic!("expected a template source, got {source:?}");
    };
    assert_eq!(
        (name.as_str(), version, task),
        ("build", Some("missing"), None)
    );
    let first = params.first_message.expect("first message");
    assert_eq!((first.text.as_str(), first.deadline_ms), ("x", None));
}

#[test]
fn program_and_template_are_exclusive_and_one_is_required() {
    let both = run_params(&with(&json!({
        "program": {"argv": ["/bin/true"]},
        "template": {"name": "build"}
    })))
    .expect("decodes");
    let error = LaunchSource::of(&both).expect_err("both named");
    assert_eq!(error, TemplateLaunchError::ProgramConflict);
    assert_eq!(
        (error.kind(), error.field_path()),
        ("invalid_params", Some("params.template"))
    );
    let neither = run_params(&base()).expect("decodes");
    let error = LaunchSource::of(&neither).expect_err("none named");
    assert_eq!(
        (error.kind(), error.field_path()),
        ("invalid_params", Some("params.program"))
    );
    let argv = run_params(&with(
        &json!({"program": {"argv": ["/bin/true", "$(touch PWNED)"]}}),
    ))
    .expect("decodes");
    let argv_source = LaunchSource::of(&argv).expect("argv");
    let expected = ["/bin/true".to_owned(), "$(touch PWNED)".to_owned()];
    assert_eq!(argv_source, LaunchSource::Argv(&expected));
}

#[test]
fn quiet_output_readiness_is_not_a_launch_field() {
    let rejected = run_params(&with(&json!({
        "template": {"name": "build", "version": "1"},
        "first_message": {"text": "x", "ready_by": "quiet"}
    })))
    .expect_err("ready_by is unknown");
    assert_eq!(rejected, "params.first_message.ready_by");
}

#[test]
fn readiness_contracts_declare_only_states_safe_for_typing() {
    let quiet: Result<ReadinessContract, _> = serde_json::from_value(json!({"by": "quiet"}));
    assert!(quiet.is_err(), "quiet is no readiness mechanism");
    let unsafe_states = [AgentState::Working, AgentState::Blocked, AgentState::Error];
    for state in unsafe_states {
        let error = ReadinessContract::declared(vec![AgentState::Idle, state]).expect_err("unsafe");
        assert_eq!(error.kind(), "template_invalid");
    }
    assert!(ReadinessContract::declared(Vec::new()).is_err());
    let contract: ReadinessContract =
        serde_json::from_value(json!({"by": "declared", "states": ["idle", "done"]}))
            .expect("declared");
    let contract = contract.validated().expect("safe states");
    assert_eq!(contract.states(), vec![AgentState::Idle, AgentState::Done]);
    let unknown: ReadinessContract =
        serde_json::from_value(json!({"by": "declared", "states": ["quiet"]})).expect("open enum");
    assert_eq!(
        unknown.validated().map_err(|error| error.kind()),
        Err("template_invalid")
    );
}

fn record(queued: u64, deadline: u64) -> FirstMessageRecord {
    let mut record = FirstMessageRecord::accepted(
        (
            OpId::new("0191f1a2-0000-7000-8000-000000000001".to_owned()),
            TerminalId::new("100".to_owned()),
        ),
        Incarnation::new(1),
        TimestampMs::new(deadline),
    );
    record.state = OpenEnum::Known(FirstMessageState::Sent);
    record.queued_seq = Some(EventSeq::new(queued));
    record
}

#[test]
fn counters_are_decimal_strings_and_numbers_are_refused() {
    let max = record(u64::MAX, u64::MAX);
    let wire = serde_json::to_value(&max).expect("encodes");
    assert_eq!(wire.get("queued_seq"), Some(&json!("18446744073709551615")));
    assert_eq!(
        wire.get("deadline_ms"),
        Some(&json!("18446744073709551615"))
    );
    assert_eq!(wire.get("state"), Some(&json!("sent")));
    let back: FirstMessageRecord = serde_json::from_value(wire.clone()).expect("decodes");
    assert_eq!(back, max);
    let mut numeric = wire;
    if let Some(object) = numeric.as_object_mut() {
        object.insert("queued_seq".to_owned(), json!(3));
    }
    assert!(serde_json::from_value::<FirstMessageRecord>(numeric).is_err());
}

#[test]
fn an_accepted_record_is_pending_and_a_reasoned_one_is_not() {
    let mut accepted = record(0, 1);
    accepted.state = OpenEnum::Known(FirstMessageState::Accepted);
    accepted.queued_seq = None;
    assert!(accepted.is_pending());
    accepted.state = OpenEnum::Known(FirstMessageState::Expired);
    accepted.reason = Some(OpenEnum::Known(FirstMessageReason::DeadlinePassed));
    assert!(!accepted.is_pending());
    let wire = serde_json::to_value(&accepted).expect("encodes");
    assert_eq!(
        (wire.get("state"), wire.get("reason")),
        (Some(&json!("expired")), Some(&json!("deadline_passed")))
    );
}

#[test]
fn a_launched_child_dies_with_its_parent_by_default_and_carries_the_message() {
    let params = run_params(&with(&json!({
        "program": {"argv": ["/bin/cat"]},
        "parent": {"terminal_id": "7", "incarnation": "2"},
        "name": "worker"
    })))
    .expect("decodes");
    let process = started_process(
        Incarnation::new(1),
        (vec!["/bin/cat".to_owned()], None),
        TimestampMs::new(5),
    );
    let launched = LaunchedTerminal::new(
        (
            TerminalId::new("100".to_owned()),
            TerminalAddress::new("local/worker".to_owned()),
            MachineId::new("machine:1".to_owned()),
        ),
        (process, EventSeq::new(9)),
        &params,
    );
    assert_eq!(launched.spawn, SpawnState::Spawned);
    let message = record(1, 2);
    let result = launched.into_result(Some(message.clone()));
    assert!(result.terminal.die_with_parent);
    assert!(!result.terminal.notify);
    assert_eq!(result.first_message, Some(message));
    assert_eq!(
        result.terminal.status.process_incarnation,
        Some(Incarnation::new(1))
    );
    assert_eq!(result.terminal.created_seq, EventSeq::new(9));
}

proptest! {
    #[test]
    fn first_message_records_round_trip(queued in any::<u64>(), deadline in any::<u64>(), sent in any::<bool>()) {
        let mut value = record(queued, deadline);
        if !sent {
            value.state = OpenEnum::Known(FirstMessageState::Cancelled);
            value.queued_seq = None;
            value.reason = Some(OpenEnum::Known(FirstMessageReason::LeaseHeld));
        }
        let text = serde_json::to_string(&value).expect("encodes");
        let back: FirstMessageRecord = serde_json::from_str(&text).expect("decodes");
        prop_assert_eq!(back, value);
    }
}
