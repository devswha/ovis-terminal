use proptest::prelude::*;
use serde_json::json;

use super::{
    MachineHealth, MirroredEvent, NotificationId, OrphanedAckParams, OrphanedParams,
    ParentIncarnation, RemoteEvent, SkewMs,
};
use crate::domain::agent_tree::OrphanNotice;
use crate::schema::ids::MachineId;
use crate::schema::methods::machine::{MachineRecord, MachineState};
use crate::schema::scalar::{EventSeq, OpenEnum, Revision, TimestampMs};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn machine(last: u8) -> MachineId {
    MachineId::new(format!(
        "machine:0191f1a2-0000-7000-8000-0000000000{last:02x}"
    ))
}

fn notice(incarnation: u64) -> Result<OrphanNotice, serde_json::Error> {
    serde_json::from_value(json!({
        "parent_machine": machine(0x21),
        "parent": {"terminal_id": "terminal:6", "incarnation": incarnation.to_string()},
        "finished_seq": u64::MAX.to_string(),
    }))
}

proptest! {
    #[test]
    fn skew_round_trips_every_signed_value(millis in any::<i64>()) {
        let text = serde_json::to_string(&SkewMs::new(millis))?;
        prop_assert_eq!(&text, &format!("\"{millis}\""));
        prop_assert_eq!(serde_json::from_str::<SkewMs>(&text)?.get(), millis);
    }

    #[test]
    fn skew_is_remote_minus_local_without_wrapping(remote in any::<u64>(), local in any::<u64>()) {
        let skew = SkewMs::between(TimestampMs::new(remote), TimestampMs::new(local));
        let exact = i128::from(remote).saturating_sub(i128::from(local));
        let clamped = exact.clamp(i128::from(i64::MIN), i128::from(i64::MAX));
        prop_assert_eq!(i128::from(skew.get()), clamped);
    }
}

#[test]
fn skew_rejects_numbers_and_non_canonical_text() {
    for text in ["5", "\"+5\"", "\"05\"", "\"-0\"", "\"1e3\"", "\"\""] {
        assert!(serde_json::from_str::<SkewMs>(text).is_err(), "{text}");
    }
}

#[test]
fn a_replayed_notice_keeps_its_id_and_each_target_gets_its_own() -> TestResult {
    let first = NotificationId::of(&machine(0xb2), &notice(1)?);
    assert_eq!(first, NotificationId::of(&machine(0xb2), &notice(1)?));
    assert_ne!(first, NotificationId::of(&machine(0xa1), &notice(1)?));
    assert_ne!(first, NotificationId::of(&machine(0xb2), &notice(2)?));
    assert_ne!(
        ParentIncarnation::of(&notice(1)?),
        ParentIncarnation::of(&notice(2)?)
    );
    Ok(())
}

#[test]
fn orphaned_params_round_trip_with_counters_as_strings() -> TestResult {
    let params = OrphanedParams::to(&machine(0xb2), notice(u64::MAX)?);
    let value = serde_json::to_value(&params)?;
    let max = json!(u64::MAX.to_string());
    assert_eq!(value.pointer("/notice/finished_seq"), Some(&max));
    assert_eq!(value.pointer("/notice/parent/incarnation"), Some(&max));
    assert_eq!(serde_json::from_value::<OrphanedParams>(value)?, params);
    let mut numeric = serde_json::to_value(&params)?;
    *numeric
        .pointer_mut("/notice/finished_seq")
        .ok_or("finished_seq")? = json!(7);
    assert!(serde_json::from_value::<OrphanedParams>(numeric).is_err());
    Ok(())
}

#[test]
fn ack_tokens_are_opaque_but_bounded_visible_ascii() {
    let ack = |id: serde_json::Value| {
        serde_json::from_value::<OrphanedAckParams>(
            json!({"notification_id": id, "parent_incarnation": "reused-old"}),
        )
    };
    assert!(ack(json!("unknown")).is_ok());
    for refused in [
        json!(""),
        json!("has space"),
        json!("x".repeat(513)),
        json!(7),
    ] {
        assert!(ack(refused.clone()).is_err(), "{refused}");
    }
    let unknown_field = json!({"notification_id": "a", "parent_incarnation": "b", "machine": "c"});
    assert!(serde_json::from_value::<OrphanedAckParams>(unknown_field).is_err());
}

#[test]
fn remote_events_refuse_numeric_counters() -> TestResult {
    let event = json!({
        "seq": u64::MAX.to_string(), "runtime_lifetime": "7", "ts_ms": "1",
        "type": "terminal.created",
        "terminal": {"terminal_id": "terminal:1", "revision": "3", "removed": false},
        "data": {"any": ["thing"]},
    });
    let decoded: RemoteEvent = serde_json::from_value(event.clone())?;
    assert_eq!(decoded.seq.get(), u64::MAX);
    assert_eq!(serde_json::to_value(&decoded)?, event);
    for field in ["seq", "ts_ms"] {
        let mut numeric = event.clone();
        *numeric.get_mut(field).ok_or("field")? = json!(1);
        assert!(
            serde_json::from_value::<RemoteEvent>(numeric).is_err(),
            "{field}"
        );
    }
    let mirrored = json!({
        "source": {"machine_id": machine(0xb2), "runtime_lifetime": "7", "seq": "1"},
        "remote_ts_ms": "1", "received_ms": "2", "clock_skew_ms": "-1",
        "type": "x", "terminal": null, "data": null,
    });
    let decoded: MirroredEvent = serde_json::from_value(mirrored.clone())?;
    assert_eq!(serde_json::to_value(decoded)?, mirrored);
    Ok(())
}

#[test]
fn an_observed_record_takes_the_line_state_and_a_later_revision() -> TestResult {
    let record: MachineRecord = serde_json::from_value(json!({
        "machine_id": machine(0xb2), "name": "bravo", "aliases": [],
        "state": "connected", "remote_lifetime": "7", "remote_boot_id": "1",
        "last_seen_seq": null, "version": "1.0.0", "revision": "4",
    }))?;
    let mut health = MachineHealth::detached(machine(0xb2));
    health.state = OpenEnum::Known(MachineState::Unreachable);
    health.revision = Revision::new(3);
    health.last_seen_seq = Some(EventSeq::new(9));
    let observed = record.observed(&health);
    assert_eq!(observed.state, OpenEnum::Known(MachineState::Unreachable));
    assert_eq!(observed.revision, Revision::new(7));
    assert_eq!(observed.last_seen_seq.map(EventSeq::get), Some(9));
    assert_eq!(observed.name, record.name);
    Ok(())
}
