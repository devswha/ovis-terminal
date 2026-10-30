//! Contract tests for redaction, structured correlation, and sink failures.

use std::io;
use std::str::FromStr as _;

use ovis_terminal_core::telemetry::{
    Counter, Counters, FieldValue, Outcome, TelemetryError, TelemetrySink, TraceContext, emit,
    log_handled_error,
};

#[derive(Default)]
struct CaptureSink {
    records: Vec<Vec<u8>>,
}

impl TelemetrySink for CaptureSink {
    fn write_record(&mut self, record: &[u8]) -> io::Result<()> {
        self.records.push(record.to_vec());
        Ok(())
    }
}

struct RejectSink;

impl TelemetrySink for RejectSink {
    fn write_record(&mut self, _: &[u8]) -> io::Result<()> {
        Err(io::Error::other("sink unavailable"))
    }
}

#[test]
fn happy() {
    let mut sink = CaptureSink::default();
    let context = TraceContext::default()
        .with_request("request-7".into())
        .with_operation("operation-9".into())
        .with_principal("principal-a".into())
        .with_machine("machine-b".into())
        .with_runtime("runtime-c".into())
        .with_resource("pane-d".into())
        .with_sequence(42)
        .with_outcome(Outcome::Success);

    emit(&mut sink, &context, Some(FieldValue::Text("hostile\ntext"))).expect("sink accepts");

    let record = sink
        .records
        .first()
        .expect("emit writes one captured record");
    let fields: serde_json::Value = serde_json::from_slice(record).expect("record is JSON");
    assert_eq!(
        fields.get("request").and_then(serde_json::Value::as_str),
        Some("request-7")
    );
    assert_eq!(
        fields.get("operation").and_then(serde_json::Value::as_str),
        Some("operation-9")
    );
    assert_eq!(
        fields.get("principal").and_then(serde_json::Value::as_str),
        Some("principal-a")
    );
    assert_eq!(
        fields.get("machine").and_then(serde_json::Value::as_str),
        Some("machine-b")
    );
    assert_eq!(
        fields.get("runtime").and_then(serde_json::Value::as_str),
        Some("runtime-c")
    );
    assert_eq!(
        fields.get("resource").and_then(serde_json::Value::as_str),
        Some("pane-d")
    );
    assert_eq!(
        fields.get("sequence").and_then(serde_json::Value::as_str),
        Some("42")
    );
    assert_eq!(
        fields.get("outcome").and_then(serde_json::Value::as_str),
        Some("success")
    );
    assert_eq!(
        fields.get("value").and_then(serde_json::Value::as_str),
        Some("hostile\ntext")
    );
}

#[test]
fn maximum_sequence_round_trips_as_decimal_string() {
    let mut sink = CaptureSink::default();
    let context = TraceContext::default().with_sequence(u64::MAX);

    emit(&mut sink, &context, None).expect("sink accepts");

    let record = sink
        .records
        .first()
        .expect("emit writes one captured record");
    let value: serde_json::Value = serde_json::from_slice(record).expect("record is JSON");
    let sequence = value
        .get("sequence")
        .and_then(serde_json::Value::as_str)
        .expect("sequence is a decimal string");

    assert_eq!(sequence, "18446744073709551615");
    assert_eq!(
        u64::from_str(sequence).expect("u64 sequence parses"),
        u64::MAX
    );
}

#[test]
fn failure() {
    let mut sink = CaptureSink::default();
    let context = TraceContext::default()
        .with_request("request-8".into())
        .with_outcome(Outcome::Failure);

    emit(
        &mut sink,
        &context,
        Some(FieldValue::Credential("SECRET_TEST_VALUE")),
    )
    .expect("sink accepts");
    emit(
        &mut sink,
        &context,
        Some(FieldValue::SecretInput("secret-input-payload")),
    )
    .expect("sink accepts");
    log_handled_error(&mut sink, &context).expect("sink accepts");

    let all_records = sink.records.concat();
    let captured = String::from_utf8_lossy(&all_records);
    assert!(!captured.contains("SECRET_TEST_VALUE"));
    assert!(!captured.contains("secret-input-payload"));
    assert_eq!(sink.records.len(), 3);
}

#[test]
fn invalid_sink_returns_typed_error() {
    let result = emit(&mut RejectSink, &TraceContext::default(), None);

    let error = result.expect_err("sink rejects the record");
    assert!(matches!(&error, TelemetryError::TelemetryIo(_)));
    assert_eq!(error.kind(), "telemetry_io");
    assert_eq!(error.hint(), "check sink path permissions and retry");
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn oversized_field_is_bounded() {
    let mut sink = CaptureSink::default();
    let value = "x".repeat(1024);
    emit(
        &mut sink,
        &TraceContext::default().with_resource(value.into()),
        None,
    )
    .expect("sink accepts");

    let record = sink
        .records
        .first()
        .expect("emit writes one captured record");
    assert!(record.len() < 300);
}

#[test]
fn counter_values_saturate_at_their_bound() {
    let mut counters = Counters::default();

    counters.add(Counter::QueueBytes, u64::MAX);
    counters.add(Counter::QueueBytes, 1);

    assert_eq!(counters.get(Counter::QueueBytes), 1_000_000);
}
