//! Shared value fixture covering all frozen v1 holder message variants.

use super::*;

pub(super) const MESSAGE_CASES: usize = 16;

#[expect(
    clippy::too_many_lines,
    reason = "one shared fixture enumerates every frozen wire message kind"
)]
pub(super) fn message_examples() -> Vec<Message> {
    let terminal_id = TerminalId::new("t-property".to_owned());
    let terminal_incarnation = TerminalIncarnation::new(4);
    let process_incarnation = ProcessIncarnation::new(9);
    let process_start = ProcessStartToken::new("start-token-a".to_owned());
    let target = TerminalTarget {
        terminal_id: terminal_id.clone(),
        terminal_incarnation,
        process_incarnation: Some(process_incarnation),
    };
    let exit = Exit {
        target: target.clone(),
        code: Some(0),
        signal: None,
        final_offset: 8,
    };
    let held = HeldTerminal {
        terminal_id: terminal_id.clone(),
        terminal_incarnation,
        process_incarnation: Some(process_incarnation),
        pid: Some(4242),
        start_token: Some(process_start.clone()),
        output_offset: 8,
        input_sequence: 3,
        exit: Some(exit.clone()),
    };
    let grant = WriteGrant {
        grant_epoch: 12,
        fence_epoch: 13,
        process_incarnation,
    };
    vec![
        Message::Hello(Hello {
            generation: 1,
            core_build: CoreBuildId::new("core-build-a".to_owned()),
            previous: PreviousCoreState::Crash,
        }),
        Message::Welcome(Welcome {
            generation: 1,
            build: HolderBuildId::new("holder-build-a".to_owned()),
            instance: HolderInstanceId::new("holder-instance-a".to_owned()),
            terminals: vec![held],
        }),
        Message::Spawn(Spawn {
            terminal_id: terminal_id.clone(),
            terminal_incarnation,
            argv: vec!["sh".to_owned()],
            geometry: (80, 24),
        }),
        Message::Spawned(Spawned {
            terminal_id: terminal_id.clone(),
            terminal_incarnation,
            process_incarnation,
            pid: 4242,
            start_token: process_start,
        }),
        Message::SpawnFailed(SpawnFailed {
            terminal_id,
            terminal_incarnation,
            kind: SpawnFailureKind::Unavailable,
        }),
        Message::Output(Output {
            target: target.clone(),
            offset: 4,
            bytes: b"tail".to_vec(),
        }),
        Message::Input(Input {
            target: target.clone(),
            sequence: 3,
            grant,
            bytes: b"echo x\n".to_vec(),
        }),
        Message::InputWritten(InputWritten {
            target: target.clone(),
            sequence: 3,
            grant_epoch: 12,
            fence_epoch: 13,
            ack_epoch: 14,
            written_bytes: 7,
            output_offset: 8,
        }),
        Message::Resize(Resize {
            target: target.clone(),
            geometry: (100, 30),
            geometry_epoch: 15,
        }),
        Message::Signal(Signal {
            target: target.clone(),
            signal: SignalKind::Terminate,
        }),
        Message::Exit(exit),
        Message::TailRequest(TailRequest {
            target: target.clone(),
            from_offset: 0,
        }),
        Message::Tail(Tail {
            target: target.clone(),
            from_offset: 0,
            through_offset: 4,
            bytes: b"head".to_vec(),
            gap: false,
        }),
        Message::Forget(Forget {
            target,
            acknowledged_final_offset: Some(8),
        }),
        Message::Drain(Drain::Begin),
        Message::Goodbye(Goodbye {
            reason: GoodbyeReason::Handover,
        }),
    ]
}
