use std::io;
use std::num::NonZeroUsize;

use ovis_terminal_protocol::schema::scalar::TimestampMs;
use proptest::prelude::*;
use serde_json::{Value, json};

use super::{AuditLog, REDACTED, RedactingSink, RetentionPolicy, redact, scrub, seal};
use crate::telemetry::{
    FieldValue, OperationId, Outcome, ResourceId, TelemetrySink, TraceContext, emit,
};

const SENTINEL: &str = "sk-live-SENTINEL-7f3a9c";

fn bearer() -> String {
    format!("ovt1.c42.{}", "0123456789abcdef".repeat(4))
}

#[derive(Debug, Default)]
struct Capture(Vec<u8>);

impl TelemetrySink for Capture {
    fn write_record(&mut self, record: &[u8]) -> io::Result<()> {
        self.0.extend_from_slice(record);
        self.0.push(b'\n');
        Ok(())
    }
}

fn occurrences(haystack: &[u8], needle: &str) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle.as_bytes())
        .count()
}

#[test]
fn a_captured_log_holds_no_secret_but_keeps_every_correlation_id() -> io::Result<()> {
    let mut log = RedactingSink::new(Capture::default());
    let context = TraceContext::default()
        .with_operation(OperationId::from("op-7"))
        .with_resource(ResourceId::from("terminal:100"))
        .with_outcome(Outcome::Failure);
    let carried = format!("presented {} for op-9", bearer());
    let records = [
        json!({"method": "machine.add", "op_id": "op-1", "params": {"credential": SENTINEL}}),
        json!({"method": "input.send", "op_id": "op-2", "params": {"text": SENTINEL}}),
        json!({"method": "terminal.run", "op_id": "op-3",
               "params": {"argv": ["/bin/echo", "a;b"], "env": {"API_KEY": SENTINEL}}}),
        json!({"method": "credential.rotate", "result": {"issued": {"bearer": bearer()}}}),
        json!({"error": {"message": format!("refused {}", bearer())}}),
    ];

    for record in &records {
        log.write_record(&serde_json::to_vec(record)?)?;
    }
    emit(&mut log, &context, Some(FieldValue::Credential(SENTINEL)))
        .map_err(|error| io::Error::other(error.kind()))?;
    emit(&mut log, &context, Some(FieldValue::Text(&carried)))
        .map_err(|error| io::Error::other(error.kind()))?;
    let captured = &log.inner().0;

    assert_eq!(occurrences(captured, SENTINEL), 0);
    assert_eq!(occurrences(captured, &bearer()), 0);
    for kept in [
        "op-1",
        "op-2",
        "op-3",
        "op-7",
        "op-9",
        "terminal:100",
        "API_KEY",
        "a;b",
    ] {
        assert_ne!(occurrences(captured, kept), 0, "{kept}");
    }
    Ok(())
}

#[test]
fn every_emitted_text_is_scrubbed_before_its_bound_cuts_a_bearer() -> io::Result<()> {
    let mut plain = Capture::default();
    let hex = "0123456789abcdef".repeat(4);
    let cut = format!("{}{}", "a".repeat(220), bearer());

    emit(
        &mut plain,
        &TraceContext::default(),
        Some(FieldValue::Text(&cut)),
    )
    .map_err(|error| io::Error::other(error.kind()))?;

    assert_eq!(occurrences(&plain.0, hex.get(..24).unwrap_or_default()), 0);
    assert_eq!(occurrences(&plain.0, REDACTED), 1);
    Ok(())
}

#[test]
fn a_record_that_is_not_json_is_refused_rather_than_passed_on() {
    let mut log = RedactingSink::new(Capture::default());

    let written = log.write_record(format!("raw {}", bearer()).as_bytes());

    assert_eq!(
        written.map_err(|error| error.kind()),
        Err(io::ErrorKind::InvalidData)
    );
    assert_eq!(log.inner().0, Vec::<u8>::new());
}

#[test]
fn environment_names_stay_diagnostic_while_their_values_are_replaced() {
    let mut record = json!({"env": {"ROLE": "build", "TOKEN": SENTINEL}, "keys": ["p", "w"]});

    redact(&mut record);

    assert_eq!(
        record,
        json!({"env": {"ROLE": REDACTED, "TOKEN": REDACTED}, "keys": REDACTED})
    );
}

#[test]
fn bearer_lookalikes_are_kept_and_real_bearers_scrubbed() {
    let hex = "0123456789abcdef".repeat(4);
    let text = format!("ovt1.c3.abc ovt1.c.{hex} ovt1.c9.{hex} tail");

    assert_eq!(
        scrub(&text),
        format!("ovt1.c3.abc ovt1.c.{hex} {REDACTED} tail")
    );
}

#[test]
fn a_sealed_secret_persists_only_as_its_sha256_digest() {
    let mut result = json!({"issued": {"bearer": "abc", "credential": "c4"}});

    let original = seal(&mut result, "/issued/bearer");
    let absent = seal(&mut result, "/issued/missing");

    assert_eq!(original.as_deref(), Some("abc"));
    assert_eq!(absent, None);
    assert_eq!(
        result.pointer("/issued/bearer"),
        Some(&json!(
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        ))
    );
}

fn policy(records: usize, age: u64) -> RetentionPolicy {
    RetentionPolicy::new(NonZeroUsize::new(records).unwrap_or(NonZeroUsize::MIN), age)
}

#[test]
fn the_audit_trail_evicts_by_count_and_by_age_and_counts_what_it_dropped() {
    let mut trail = AuditLog::new(policy(2, 1_000));
    for (at, op) in [(10, "a"), (20, "b"), (30, "c")] {
        trail.append((TimestampMs::new(at), json!({"op_id": op})));
    }
    let by_count = trail.records(TimestampMs::new(30));
    let by_age = trail.records(TimestampMs::new(1_025));

    assert_eq!(
        by_count,
        [
            json!({"op_id": "b", "seq": "2"}),
            json!({"op_id": "c", "seq": "3"})
        ]
    );
    assert_eq!(by_age, [json!({"op_id": "c", "seq": "3"})]);
    assert_eq!(trail.evicted(), 2);
}

#[test]
fn the_audit_trail_retains_only_redacted_records() {
    let mut trail = AuditLog::new(policy(8, 1_000));

    trail.append((TimestampMs::new(1), json!({"params": {"answer": SENTINEL}})));
    let retained = serde_json::to_string(&trail.records(TimestampMs::new(1))).unwrap_or_default();

    assert!(!retained.contains(SENTINEL));
    assert!(retained.contains(REDACTED));
}

proptest! {
    #[test]
    fn a_secret_field_is_redacted_at_any_depth(
        secret in "[A-Za-z0-9]{16,32}",
        depth in 0_usize..6,
        field in prop::sample::select(vec!["text", "bearer", "credential", "token", "data_b64"]),
    ) {
        let mut record = json!({ field: &secret });
        for level in 0..depth {
            record = json!({ format!("level{level}"): [record] });
        }

        redact(&mut record);

        prop_assert!(!record.to_string().contains(&secret));
        let mut untouched = Value::String(secret.clone());
        redact(&mut untouched);
        prop_assert_eq!(untouched, Value::String(secret));
    }
}
