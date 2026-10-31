//! Terminal lifecycle compatibility and process evidence.

use ovis_terminal_holder::protocol::{FrameHeader, Message, MessageKind, decode_message};

#[cfg(unix)]
#[path = "../src/terminals/qa/mod.rs"]
pub mod qa;

#[cfg(unix)]
#[test]
fn happy() -> qa::TestResult {
    qa::happy()
}

#[cfg(unix)]
#[test]
fn failure() -> qa::TestResult {
    qa::failure()
}

#[cfg(unix)]
#[test]
fn holder_publishes_hangup_before_exit() {
    use std::time::Duration;

    use ovis_terminal_holder::protocol::{
        CoreBuildId, HOLDER_PROTOCOL_GENERATION, Hello, HolderBuildId, HolderDescriptor,
        HolderInstanceId, PreviousCoreState, Spawn, TerminalId, TerminalIncarnation,
        encode_message,
    };
    use ovis_terminal_holder::unix::{Holder, HolderConfig, HolderLimits};

    // Given: a real holder and an adopted reader registered before a quick exit.
    let holder = Holder::new(HolderConfig::new(
        HolderDescriptor::new(
            HolderBuildId::new("task-16".into()),
            HolderInstanceId::new(uuid::Uuid::now_v7().to_string()),
        ),
        HolderLimits::DEFAULT,
    ));
    let link = holder
        .adopt(&Hello {
            generation: HOLDER_PROTOCOL_GENERATION,
            core_build: CoreBuildId::new("task-16".into()),
            previous: PreviousCoreState::Crash,
        })
        .expect("adopt real holder");
    let request = Spawn {
        terminal_id: TerminalId::new("terminal:1".into()),
        terminal_incarnation: TerminalIncarnation::new(1),
        argv: vec!["/bin/sh".into(), "-c".into(), "printf final; exit 7".into()],
        cwd: std::env::temp_dir(),
        env: [("PATH".into(), "/usr/bin:/bin".into())].into(),
        geometry: (80, 24),
    };

    // When: the real program writes its final bytes and exits.
    let spawned = holder.spawn_wire(&request).expect("spawn fixture");
    let mut hung_up = false;
    let mut bytes = Vec::new();
    let exit = loop {
        let message = link
            .recv_timeout(Duration::from_secs(10))
            .expect("bounded holder event");
        let encoded = encode_message(&message).expect("frame");
        let payload: serde_json::Value =
            serde_json::from_slice(encoded.get(12..).expect("payload")).expect("wire payload");
        if let Some(hangup) = payload.get("hangup") {
            assert!(!hung_up, "one hangup per process");
            assert_eq!(
                hangup.get("pid").and_then(serde_json::Value::as_str),
                Some(spawned.pid.to_string().as_str())
            );
            assert_eq!(
                hangup
                    .get("start_token")
                    .and_then(serde_json::Value::as_str),
                Some(spawned.start_token.as_str())
            );
            hung_up = true;
        }
        match message {
            Message::Output(output) => bytes.extend(output.bytes),
            Message::Exit(exit) => break exit,
            _ => {}
        }
    };
    holder.shutdown().expect("join owned workers");

    // Then: Hangup is distinct from reaped Exit, which covers all final bytes.
    assert!(hung_up, "missing distinct pre-reap hangup evidence");
    assert_eq!(bytes, b"final");
    assert_eq!(exit.code, Some(7));
    assert_eq!(exit.final_offset, 5);
}

#[test]
fn generation_two_ignores_optional_envelope_evidence() {
    // Given: an unchanged generation-two Output with optional envelope evidence.
    let payload = br#"{"kind":"output","data":{"target":{"terminal_id":"terminal:1","terminal_incarnation":"1","process_incarnation":"1"},"offset":"0","bytes":[]},"hangup":{"pid":"1","start_token":"fixture"}}"#;
    let mut frame = FrameHeader::encode(MessageKind::Output, payload.len())
        .expect("bounded header")
        .to_vec();
    frame.extend_from_slice(payload);

    // When: today's decoder receives the extended envelope.
    let decoded = decode_message(&frame).expect("additive envelope is ignored");

    // Then: an old reader retains the output boundary without claiming exit.
    let Message::Output(output) = decoded else {
        panic!("an optional envelope field must not change the legacy message");
    };
    assert_eq!(output.bytes, Vec::<u8>::new());
    assert_eq!(output.offset, 0);
}

#[test]
fn generation_two_rejects_nested_lifecycle_extension() {
    // Given: the same extension inside the closed Output payload instead.
    let payload = br#"{"kind":"output","data":{"target":{"terminal_id":"terminal:1","terminal_incarnation":"1","process_incarnation":"1"},"offset":"0","bytes":[],"hangup":{"pid":"1","start_token":"fixture"}}}"#;
    let mut frame = FrameHeader::encode(MessageKind::Output, payload.len())
        .expect("bounded header")
        .to_vec();
    frame.extend_from_slice(payload);

    // When: today's decoder receives a new nested field.
    let rejection = decode_message(&frame).expect_err("closed payload rejects the extension");

    // Then: this route is not backward-compatible.
    assert_eq!(rejection.kind(), "malformed_payload");
}
