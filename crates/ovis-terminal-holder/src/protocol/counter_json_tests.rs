//! Wire-value checks for every counter-bearing holder message.

use serde_json::Value;

use super::property_message_examples::{MESSAGE_CASES, message_examples};
use super::*;

const ABOVE_SAFE_INTEGER: u64 = 9_007_199_254_740_993;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const COUNTER_FIELDS: &[&str] = &[
    "ack_epoch",
    "acknowledged_final_offset",
    "fence_epoch",
    "final_offset",
    "from_offset",
    "generation",
    "grant_epoch",
    "geometry_epoch",
    "input_sequence",
    "limit_bytes",
    "limit_entries",
    "offset",
    "output_offset",
    "payload_len",
    "pid",
    "process_incarnation",
    "queued_bytes",
    "queued_entries",
    "sequence",
    "start_token",
    "terminal_incarnation",
    "through_offset",
    "written_bytes",
];

fn raise_counters(message: &mut Message) {
    match message {
        Message::Hello(hello) => hello.generation = u16::MAX,
        Message::Welcome(welcome) => {
            welcome.generation = u16::MAX;
            for terminal in &mut welcome.terminals {
                raise_terminal_counters(terminal);
            }
        }
        Message::Spawn(spawn) => {
            spawn.terminal_incarnation = TerminalIncarnation::new(ABOVE_SAFE_INTEGER);
        }
        Message::Spawned(spawned) => {
            spawned.terminal_incarnation = TerminalIncarnation::new(ABOVE_SAFE_INTEGER);
            spawned.process_incarnation = ProcessIncarnation::new(ABOVE_SAFE_INTEGER);
            spawned.pid = u32::MAX;
        }
        Message::SpawnFailed(spawn_failed) => {
            spawn_failed.terminal_incarnation = TerminalIncarnation::new(ABOVE_SAFE_INTEGER);
        }
        Message::Output(output) => {
            raise_target_counters(&mut output.target);
            output.offset = ABOVE_SAFE_INTEGER;
        }
        Message::Input(input) => {
            raise_target_counters(&mut input.target);
            input.sequence = ABOVE_SAFE_INTEGER;
            input.grant.grant_epoch = ABOVE_SAFE_INTEGER;
            input.grant.fence_epoch = ABOVE_SAFE_INTEGER;
            input.grant.process_incarnation = ProcessIncarnation::new(ABOVE_SAFE_INTEGER);
        }
        Message::InputWritten(input_written) => {
            raise_target_counters(&mut input_written.target);
            input_written.sequence = ABOVE_SAFE_INTEGER;
            input_written.grant_epoch = ABOVE_SAFE_INTEGER;
            input_written.fence_epoch = ABOVE_SAFE_INTEGER;
            input_written.ack_epoch = ABOVE_SAFE_INTEGER;
            input_written.written_bytes = ABOVE_SAFE_INTEGER;
            input_written.output_offset = ABOVE_SAFE_INTEGER;
        }
        Message::Resize(resize) => {
            raise_target_counters(&mut resize.target);
            resize.geometry_epoch = ABOVE_SAFE_INTEGER;
        }
        Message::Signal(signal) => raise_target_counters(&mut signal.target),
        Message::Exit(exit) => {
            raise_target_counters(&mut exit.target);
            exit.final_offset = ABOVE_SAFE_INTEGER;
        }
        Message::TailRequest(tail) => {
            raise_target_counters(&mut tail.target);
            tail.from_offset = ABOVE_SAFE_INTEGER;
        }
        Message::Tail(tail) => {
            raise_target_counters(&mut tail.target);
            tail.from_offset = ABOVE_SAFE_INTEGER;
            tail.through_offset = ABOVE_SAFE_INTEGER + 1;
        }
        Message::Forget(forget) => {
            raise_target_counters(&mut forget.target);
            forget.acknowledged_final_offset = Some(ABOVE_SAFE_INTEGER);
        }
        Message::InputRejected(rejected) => {
            raise_target_counters(&mut rejected.target);
            rejected.sequence = ABOVE_SAFE_INTEGER;
            rejected.queued_bytes = ABOVE_SAFE_INTEGER;
            rejected.queued_entries = ABOVE_SAFE_INTEGER;
            rejected.limit_bytes = u64::MAX;
            rejected.limit_entries = u64::MAX;
        }
        Message::Drain(_) | Message::Goodbye(_) => {}
    }
}

fn raise_terminal_counters(terminal: &mut HeldTerminal) {
    terminal.terminal_incarnation = TerminalIncarnation::new(ABOVE_SAFE_INTEGER);
    terminal.process_incarnation = Some(ProcessIncarnation::new(ABOVE_SAFE_INTEGER));
    terminal.pid = Some(u32::MAX);
    terminal.output_offset = ABOVE_SAFE_INTEGER;
    terminal.input_sequence = ABOVE_SAFE_INTEGER;
    if let Some(exit) = &mut terminal.exit {
        raise_target_counters(&mut exit.target);
        exit.final_offset = ABOVE_SAFE_INTEGER;
    }
}

fn raise_target_counters(target: &mut TerminalTarget) {
    target.terminal_incarnation = TerminalIncarnation::new(ABOVE_SAFE_INTEGER);
    target.process_incarnation = Some(ProcessIncarnation::new(ABOVE_SAFE_INTEGER));
}

fn inspect_json(value: &Value) {
    match value {
        Value::Array(values) => {
            for item in values {
                inspect_json(item);
            }
        }
        Value::Object(fields) => {
            for (key, item) in fields {
                if COUNTER_FIELDS.contains(&key.as_str()) {
                    assert!(
                        item.is_null() || item.as_str().is_some(),
                        "counter field {key} must be a decimal string or null"
                    );
                }
                inspect_json(item);
            }
        }
        Value::Number(number) => {
            let too_large = number
                .as_u64()
                .is_some_and(|value| value > MAX_SAFE_INTEGER)
                || number
                    .as_i64()
                    .is_some_and(|value| value.unsigned_abs() > MAX_SAFE_INTEGER);
            assert!(
                !too_large,
                "JSON number exceeds the exact JavaScript integer range"
            );
        }
        Value::Null | Value::Bool(_) | Value::String(_) => {}
    }
}

#[test]
fn every_message_variant_encodes_large_counters_as_exact_json_strings() {
    let mut messages = message_examples();
    assert_eq!(messages.len(), MESSAGE_CASES);
    for message in &mut messages {
        raise_counters(message);
        let frame = encode_message(message).expect("large-counter message encodes");
        let (header, header_len) = parse_header(&frame).expect("header parses");
        let payload_end = header_len
            .checked_add(usize::try_from(header.payload_len).expect("payload fits usize"))
            .expect("frame extent fits usize");
        let payload = frame
            .get(header_len..payload_end)
            .expect("payload follows its header");
        let json: Value = serde_json::from_slice(payload).expect("payload is JSON");
        inspect_json(&json);
        assert_eq!(decode_message(&frame).expect("frame decodes"), *message);
    }
}
