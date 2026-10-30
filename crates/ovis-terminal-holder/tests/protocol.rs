//! Holder adoption and frame boundary integration tests.

use ovis_terminal_holder::protocol::{
    AdoptionTable, CoreBuildId, FrameHeader, HOLDER_PROTOCOL_GENERATION, HeldTerminal, Hello,
    HolderBuildId, HolderDescriptor, HolderInstanceId, Message, MessageKind, PreviousCoreState,
    ProcessIncarnation, ProcessStartToken, Tail, TailRequest, TerminalId, TerminalIncarnation,
    TerminalTarget, decode_message, encode_message, negotiate, parse_header,
};

#[test]
fn happy() {
    let terminal_id = TerminalId::new("t-17".to_owned());
    let terminal_incarnation = TerminalIncarnation::new(4);
    let process_incarnation = ProcessIncarnation::new(9);
    let exit = ovis_terminal_holder::protocol::Exit {
        target: TerminalTarget {
            terminal_id: terminal_id.clone(),
            terminal_incarnation,
            process_incarnation: Some(process_incarnation),
        },
        code: Some(0),
        signal: None,
        exit_unknown: None,
        io_failure: None,
        final_offset: 8,
    };
    let retained = HeldTerminal {
        terminal_id,
        terminal_incarnation,
        process_incarnation: Some(process_incarnation),
        pid: Some(4242),
        start_token: Some(ProcessStartToken::new("proc-start-abc".to_owned())),
        output_offset: 8,
        input_sequence: 3,
        exit: Some(exit.clone()),
    };
    let mut table = AdoptionTable::default();
    table.retain(retained).expect("holder record is valid");
    let hello = Hello {
        generation: HOLDER_PROTOCOL_GENERATION,
        core_build: CoreBuildId::new("core-build-2".to_owned()),
        previous: PreviousCoreState::Crash,
    };
    let holder = HolderDescriptor::new(
        HolderBuildId::new("holder-build-1".to_owned()),
        HolderInstanceId::new("holder-instance-a".to_owned()),
    );
    let welcome = negotiate(&hello, &holder, &table).expect("supported generation adopts");
    let adopted = welcome.terminals.first().expect("record is adopted");
    assert_eq!(adopted.pid, Some(4242));
    assert_eq!(
        adopted.start_token.as_ref().map(ProcessStartToken::as_str),
        Some("proc-start-abc")
    );
    assert_eq!(
        adopted.exit.as_ref().map(|record| record.final_offset),
        Some(8)
    );

    let tail = Message::Tail(Tail {
        target: exit.target.clone(),
        from_offset: 0,
        through_offset: 4,
        bytes: b"head".to_vec(),
        gap: false,
    });
    assert_eq!(
        decode_message(&encode_message(&tail).expect("tail encodes")).expect("tail frame decodes"),
        tail
    );
    let replay = negotiate(&hello, &holder, &table).expect("unacknowledged exit replays");
    assert_eq!(
        replay
            .terminals
            .first()
            .and_then(|record| record.exit.as_ref())
            .map(|record| record.final_offset),
        Some(8)
    );
    assert_eq!(
        table
            .acknowledge_exit(&exit.target, 7)
            .map_err(|error| error.kind()),
        Err("exit_not_drained")
    );
    assert_eq!(table.acknowledge_exit(&exit.target, 8), Ok(()));
    assert_eq!(table.held_terminals(), Vec::<HeldTerminal>::new());
}

#[test]
fn failure() {
    let terminal_id = TerminalId::new("t-reused".to_owned());
    let retained = HeldTerminal {
        terminal_id,
        terminal_incarnation: TerminalIncarnation::new(2),
        process_incarnation: Some(ProcessIncarnation::new(3)),
        pid: Some(100),
        start_token: Some(ProcessStartToken::new("start-a".to_owned())),
        output_offset: 6,
        input_sequence: 2,
        exit: None,
    };
    let mut table = AdoptionTable::default();
    table.retain(retained.clone()).expect("record is valid");
    let holder = HolderDescriptor::new(
        HolderBuildId::new("holder-build".to_owned()),
        HolderInstanceId::new("holder-instance".to_owned()),
    );
    let wrong_generation = Hello {
        generation: 999,
        core_build: CoreBuildId::new("core-build".to_owned()),
        previous: PreviousCoreState::Crash,
    };
    let error = negotiate(&wrong_generation, &holder, &table)
        .expect_err("wrong generation is rejected before adoption");
    assert_eq!(error.kind(), "unsupported_generation");
    assert_ne!(error.hint(), "");
    assert_eq!(table.held_terminals(), vec![retained.clone()]);

    let stale = HeldTerminal {
        terminal_incarnation: TerminalIncarnation::new(3),
        ..retained.clone()
    };
    assert_eq!(
        table.retain(stale).expect_err("incarnation changed").kind(),
        "stale_target"
    );
    assert_eq!(table.held_terminals(), vec![retained]);

    let mut bad_version = FrameHeader::encode(MessageKind::Hello, 0).expect("header fits");
    bad_version[4..6].copy_from_slice(&999_u16.to_le_bytes());
    assert_eq!(
        parse_header(&bad_version)
            .expect_err("wrong version")
            .kind(),
        "unsupported_generation"
    );
    let mut bad_flags = FrameHeader::encode(MessageKind::Hello, 0).expect("header fits");
    bad_flags[7] = 1;
    assert_eq!(
        parse_header(&bad_flags)
            .expect_err("reserved flags are rejected")
            .kind(),
        "malformed_frame"
    );
    assert_eq!(
        decode_message(&[0, 1, 2]).expect_err("short frame").kind(),
        "truncated_frame"
    );
    assert_eq!(
        FrameHeader::encode(MessageKind::Output, 16 * 1024 * 1024 + 1)
            .expect_err("oversized payload is rejected")
            .kind(),
        "frame_too_large"
    );

    let unsupported = Message::TailRequest(TailRequest {
        target: TerminalTarget {
            terminal_id: TerminalId::new("t-reused".to_owned()),
            terminal_incarnation: TerminalIncarnation::new(2),
            process_incarnation: Some(ProcessIncarnation::new(3)),
        },
        from_offset: 0,
    });
    assert_eq!(
        decode_message(&encode_message(&unsupported).expect("tail request encodes"))
            .expect("tail request decodes"),
        unsupported
    );
}

#[test]
fn adoption_transition_table_returns_typed_kinds() {
    let terminal_id = TerminalId::new("t-transition".to_owned());
    let holder = HolderDescriptor::new(
        HolderBuildId::new("holder-build".to_owned()),
        HolderInstanceId::new("holder-instance".to_owned()),
    );
    let hello = Hello {
        generation: HOLDER_PROTOCOL_GENERATION,
        core_build: CoreBuildId::new("core-build".to_owned()),
        previous: PreviousCoreState::Handover,
    };
    let mut table = AdoptionTable::default();
    assert_eq!(
        negotiate(&hello, &holder, &table)
            .expect("fresh adoption succeeds")
            .terminals,
        Vec::<HeldTerminal>::new()
    );
    let retained = HeldTerminal {
        terminal_id,
        terminal_incarnation: TerminalIncarnation::new(7),
        process_incarnation: Some(ProcessIncarnation::new(6)),
        pid: Some(70),
        start_token: Some(ProcessStartToken::new("start-70".to_owned())),
        output_offset: 0,
        input_sequence: 0,
        exit: None,
    };
    table
        .retain(retained.clone())
        .expect("new identity is retained");
    assert_eq!(
        negotiate(&hello, &holder, &table)
            .expect("same incarnation re-adopts")
            .terminals,
        vec![retained.clone()]
    );
    let stale = HeldTerminal {
        terminal_incarnation: TerminalIncarnation::new(8),
        ..retained.clone()
    };
    assert_eq!(
        table
            .retain(stale)
            .expect_err("old identity rejected")
            .kind(),
        "stale_target"
    );
    let wrong_generation = Hello {
        generation: 999,
        ..hello
    };
    assert_eq!(
        negotiate(&wrong_generation, &holder, &table)
            .expect_err("unsupported generation rejected")
            .kind(),
        "unsupported_generation"
    );
    assert_eq!(table.held_terminals(), vec![retained]);
}
