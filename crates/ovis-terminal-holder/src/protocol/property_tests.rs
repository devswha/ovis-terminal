//! Property coverage for every v1 holder message kind.

use proptest::prelude::*;
use proptest::test_runner::{Config, FileFailurePersistence};
use serde_json::Value;

use super::property_message_examples::{MESSAGE_CASES, message_examples};
use super::*;

#[test]
fn all_message_kinds_round_trip_at_every_two_fragment_boundary() {
    let messages = message_examples();
    assert_eq!(messages.len(), MESSAGE_CASES);
    for message in messages {
        let frame = encode_message(&message).expect("typed message encodes");
        for boundary in 0..=frame.len() {
            let mut frame_reader = FrameDecoder::default();
            let mut decoded_messages = frame_reader
                .push(
                    frame
                        .get(..boundary)
                        .expect("first fragment boundary is valid"),
                )
                .expect("first fragment is accepted");
            decoded_messages.extend(
                frame_reader
                    .push(
                        frame
                            .get(boundary..)
                            .expect("second fragment boundary is valid"),
                    )
                    .expect("second fragment is accepted"),
            );
            assert_eq!(decoded_messages, vec![message.clone()]);
            frame_reader
                .finish()
                .expect("decoder ends at a frame boundary");
        }
    }
}

#[test]
fn rejected_header_does_not_leave_a_partial_decoder_frame() {
    let mut decoder = FrameDecoder::default();
    let mut malformed = FrameHeader::encode(MessageKind::Hello, 0).expect("header fits");
    malformed[0] = b'X';
    assert_eq!(
        decoder
            .push(&malformed)
            .expect_err("wrong magic is rejected")
            .kind(),
        "malformed_frame"
    );

    let next = Message::Drain(Drain::Begin);
    assert_eq!(
        decoder
            .push(&encode_message(&next).expect("drain encodes"))
            .expect("next frame decodes"),
        vec![next]
    );
}

proptest! {
    #![proptest_config(Config {
        failure_persistence: Some(Box::new(FileFailurePersistence::WithSource("proptest-regressions"))),
        ..Config::default()
    })]

    #[test]
    fn arbitrary_fragment_widths_round_trip(
        message_index in 0usize..MESSAGE_CASES,
        widths in prop::collection::vec(1u8..=32, 0..64),
    ) {
        let messages = message_examples();
        let message = messages.get(message_index).expect("message index is in range");
        let frame = encode_message(message).expect("typed message encodes");
        let mut remaining = frame.as_slice();
        let mut frame_reader = FrameDecoder::default();
        let mut decoded_messages = Vec::new();
        for width in widths {
            if remaining.is_empty() {
                break;
            }
            let take = usize::from(width).min(remaining.len());
            let (fragment, rest) = remaining.split_at(take);
            decoded_messages.extend(frame_reader.push(fragment).expect("fragment is accepted"));
            remaining = rest;
        }
        decoded_messages.extend(frame_reader.push(remaining).expect("final fragment is accepted"));
        prop_assert_eq!(decoded_messages, vec![message.clone()]);
        prop_assert!(frame_reader.finish().is_ok());
    }

    #[test]
    fn identity_fields_round_trip_as_canonical_decimal_strings(
        terminal_id in "[a-z0-9-]{1,16}",
        terminal_incarnation in any::<u64>(),
        process_incarnation in any::<u64>(),
        from_offset in any::<u64>(),
    ) {
        let message = Message::TailRequest(TailRequest {
            target: TerminalTarget {
                terminal_id: TerminalId::new(terminal_id),
                terminal_incarnation: TerminalIncarnation::new(terminal_incarnation),
                process_incarnation: Some(ProcessIncarnation::new(process_incarnation)),
            },
            from_offset,
        });
        let frame = encode_message(&message).expect("identity message encodes");
        let decoded = decode_message(&frame).expect("identity message decodes");
        prop_assert_eq!(&decoded, &message);
        let json: Value = serde_json::to_value(&message).expect("message becomes protocol JSON");
        let target = json
            .get("data")
            .and_then(|data| data.get("target"))
            .expect("tail request target is encoded");
        let expected_terminal_incarnation = terminal_incarnation.to_string();
        let expected_process_incarnation = process_incarnation.to_string();
        let expected_from_offset = from_offset.to_string();
        prop_assert_eq!(
            target.get("terminal_incarnation").and_then(Value::as_str),
            Some(expected_terminal_incarnation.as_str())
        );
        prop_assert_eq!(
            target.get("process_incarnation").and_then(Value::as_str),
            Some(expected_process_incarnation.as_str())
        );
        prop_assert_eq!(
            json.get("data")
                .and_then(|data| data.get("from_offset"))
                .and_then(Value::as_str),
            Some(expected_from_offset.as_str())
        );
    }
}

#[test]
fn frame_errors_expose_stable_kinds_hints_and_sources() {
    let malformed = decode_message(&[0, 1, 2]).expect_err("incomplete frame is rejected");
    assert_eq!(malformed.kind(), "truncated_frame");
    assert_ne!(malformed.hint(), "");

    let header = FrameHeader::encode(MessageKind::Hello, 1).expect("header fits");
    let mut malformed_payload = header.to_vec();
    malformed_payload.push(b'{');
    let error = decode_message(&malformed_payload).expect_err("invalid JSON payload is rejected");
    assert_eq!(error.kind(), "malformed_payload");
    assert!(std::error::Error::source(&error).is_some());
    assert_ne!(error.hint(), "");
}
