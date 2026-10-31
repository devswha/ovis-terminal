//! The generation-two payload remains closed; only its envelope gains evidence.

use super::property_message_examples::message_examples;
use super::{FrameDecoder, Message, decode_message, encode_message};

#[test]
fn legacy_generation_two_reader_ignores_hangup_without_losing_output() {
    // Given: the complete generation-two fixture set and one extended Output.
    for message in message_examples() {
        let bytes = encode_message(&message).expect("frame");
        let payload = bytes.get(12..).expect("payload");

        // When: the legacy adjacent-tagged Message reader ignores envelope fields.
        // Its closed payload schema is unchanged; hangup is skipped by that schema.
        let legacy: Message = serde_json::from_slice(payload).expect("legacy generation two");
        let upgraded = decode_message(&bytes).expect("upgraded generation two");

        // Then: all existing records replay unchanged; the extension adds no bytes.
        assert_eq!(upgraded, message);
        match (&message, legacy) {
            (Message::Output(sent), Message::Output(received)) => {
                assert_eq!(received.target, sent.target);
                assert_eq!(received.offset, sent.offset);
                assert_eq!(received.bytes, sent.bytes);
                assert_eq!(received.hangup, None);
            }
            (expected, actual) => assert_eq!(actual, *expected),
        }
    }
}

#[test]
fn maximum_offset_hangup_survives_every_fragment_boundary() {
    // Given: the fixture carries u64::MAX, beyond exact JSON-number precision.
    let message = message_examples()
        .into_iter()
        .find(|message| matches!(message, Message::Output(output) if output.hangup.is_some()))
        .expect("hangup fixture");
    let frame = encode_message(&message).expect("frame");
    let payload: serde_json::Value =
        serde_json::from_slice(frame.get(12..).expect("payload")).expect("JSON");
    assert_eq!(
        payload.get("data").and_then(|data| data.get("offset")),
        Some(&serde_json::Value::String(u64::MAX.to_string()))
    );

    // When: every possible two-fragment split is delivered.
    for split in 0..=frame.len() {
        let (first, second) = frame.split_at(split);
        let mut decoder = FrameDecoder::default();
        let mut received = decoder.push(first).expect("first fragment");
        received.extend(decoder.push(second).expect("second fragment"));

        // Then: the full typed record and exact counter are retained.
        assert_eq!(received, vec![message.clone()]);
        decoder.finish().expect("complete stream");
    }
}

#[test]
fn hangup_rejects_numeric_pid_at_the_frame_boundary() {
    // Given: a valid extended frame, but a numeric PID instead of canonical text.
    let payload = br#"{"kind":"output","data":{"target":{"terminal_id":"terminal:1","terminal_incarnation":"1","process_incarnation":"1"},"offset":"0","bytes":[]},"hangup":{"pid":1,"start_token":"fixture"}}"#;
    let mut frame = super::FrameHeader::encode(super::MessageKind::Output, payload.len())
        .expect("header")
        .to_vec();
    frame.extend_from_slice(payload);

    // When: the boundary decodes the untrusted extension.
    let error = decode_message(&frame).expect_err("numeric identity");

    // Then: it rejects the input rather than silently dropping the evidence.
    assert_eq!(error.kind(), "malformed_payload");
    assert!(std::error::Error::source(&error).is_some());
}
