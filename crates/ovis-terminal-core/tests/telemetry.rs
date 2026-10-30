//! Contract tests for redaction, structured correlation, and sink failures.

use std::io;

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

    let record = String::from_utf8_lossy(
        sink.records
            .first()
            .expect("emit writes one captured record"),
    );
    assert!(record.contains("\"request\":\"request-7\""));
    assert!(record.contains("\"operation\":\"operation-9\""));
    assert!(record.contains("\"principal\":\"principal-a\""));
    assert!(record.contains("\"machine\":\"machine-b\""));
    assert!(record.contains("\"runtime\":\"runtime-c\""));
    assert!(record.contains("\"resource\":\"pane-d\""));
    assert!(record.contains("\"sequence\":42"));
    assert!(record.contains("\"outcome\":\"success\""));
    assert!(record.contains("hostile\\ntext"));
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

    assert!(matches!(result, Err(TelemetryError::TelemetryIo(_))));
    assert_eq!(result.err().map(|error| error.kind()), Some("telemetry_io"));
}

#[test]
fn oversized_field_is_bounded() {
    let mut sink = CaptureSink::default();
    let value = "x".repeat(1024);
    emit(
        &mut sink,
        &TraceContext::default().with_resource(value),
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
