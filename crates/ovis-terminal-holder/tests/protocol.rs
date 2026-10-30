//! Public holder protocol behavior tests.

use ovis_terminal_holder::protocol::{
    AdoptionError, AdoptionTable, CoreBuildId, FrameError, FrameHeader, HeldTerminal, Hello,
    HolderBuildId, HolderDescriptor, HolderInstanceId, InputWritten, MessageKind,
    PreviousCoreState, ProcessIncarnation, ProcessStartToken, Tail, TerminalId,
    TerminalIncarnation, TerminalTarget, negotiate, parse_header,
};

#[test]
fn happy() {
    let terminal_id = TerminalId::new("t-17".to_owned());
    let terminal_incarnation = TerminalIncarnation::new(4);
    let process_incarnation = ProcessIncarnation::new(9);
    let final_exit = ovis_terminal_holder::protocol::Exit {
        target: TerminalTarget {
            terminal_id: terminal_id.clone(),
            terminal_incarnation,
            process_incarnation: Some(process_incarnation),
        },
        code: Some(0),
        signal: None,
        final_offset: 8,
    };
    let record = HeldTerminal {
        terminal_id: terminal_id.clone(),
        terminal_incarnation,
        process_incarnation: Some(process_incarnation),
        pid: Some(4242),
        start_token: Some(ProcessStartToken::new("proc-start-abc".to_owned())),
        output_offset: 8,
        input_sequence: 3,
        exit: Some(final_exit.clone()),
    };
    let mut records = AdoptionTable::default();
    assert_eq!(records.retain(record.clone()), Ok(()));

    let hello = Hello {
        generation: 1,
        core_build: CoreBuildId::new("core-build-2".to_owned()),
        previous: PreviousCoreState::Crash,
    };
    let holder = HolderDescriptor::new(
        HolderBuildId::new("holder-build-1".to_owned()),
        HolderInstanceId::new("holder-instance-a".to_owned()),
    );
    let welcome = negotiate(&hello, &holder, &records).expect("supported generation adopts");
    assert_eq!(welcome.generation, 1);
    assert_eq!(welcome.build, holder.build);
    assert_eq!(welcome.instance, holder.instance);
    assert_eq!(welcome.terminals, vec![record.clone()]);
    let adopted = welcome.terminals.first().expect("one retained terminal");
    assert_eq!(adopted.pid, Some(4242));
    assert_eq!(
        adopted.start_token,
        Some(ProcessStartToken::new("proc-start-abc".to_owned()))
    );

    let tail = Tail {
        target: final_exit.target.clone(),
        from_offset: 0,
        through_offset: 4,
        bytes: b"head".to_vec(),
        gap: false,
    };
    assert_eq!(tail.bytes, b"head");
    assert_eq!((tail.from_offset, tail.through_offset), (0, 4));

    let replay = negotiate(&hello, &holder, &records).expect("unacknowledged exit replays");
    assert_eq!(
        replay
            .terminals
            .first()
            .expect("one retained terminal")
            .exit,
        Some(final_exit)
    );
    let acknowledged_target = adopted.exit.as_ref().expect("retained exit").target.clone();
    assert_eq!(
        records.acknowledge_exit(&acknowledged_target, 7),
        Err(AdoptionError::ExitNotDrained {
            terminal_id: terminal_id.clone(),
            final_offset: 8,
        })
    );
    assert_eq!(records.held_terminals(), vec![record]);

    let target = TerminalTarget {
        terminal_id,
        terminal_incarnation,
        process_incarnation: Some(process_incarnation),
    };
    assert_eq!(records.acknowledge_exit(&target, 8), Ok(()));
    assert_eq!(records.held_terminals(), Vec::<HeldTerminal>::new());
}

#[test]
fn message_tags_and_header_bytes() {
    let encoded = FrameHeader::encode(MessageKind::Output, 0);
    assert_eq!(encoded, Ok([79, 86, 72, 80, 1, 0, 6, 0, 0, 0, 0, 0]));
}

#[test]
fn write_boundary_ack_carries_epochs() {
    let ack = InputWritten {
        target: TerminalTarget {
            terminal_id: TerminalId::new("t-write".to_owned()),
            terminal_incarnation: TerminalIncarnation::new(5),
            process_incarnation: Some(ProcessIncarnation::new(8)),
        },
        sequence: 11,
        grant_epoch: 12,
        fence_epoch: 13,
        ack_epoch: 14,
        written_bytes: 4,
        output_offset: 29,
    };
    assert_eq!(
        (
            ack.sequence,
            ack.grant_epoch,
            ack.fence_epoch,
            ack.ack_epoch
        ),
        (11, 12, 13, 14)
    );
    assert_eq!((ack.written_bytes, ack.output_offset), (4, 29));
}

#[test]
fn failure() {
    let terminal_id = TerminalId::new("t-reused".to_owned());
    let retained = HeldTerminal {
        terminal_id: terminal_id.clone(),
        terminal_incarnation: TerminalIncarnation::new(2),
        process_incarnation: Some(ProcessIncarnation::new(3)),
        pid: Some(100),
        start_token: Some(ProcessStartToken::new("start-a".to_owned())),
        output_offset: 6,
        input_sequence: 2,
        exit: None,
    };
    let mut records = AdoptionTable::default();
    assert_eq!(records.retain(retained.clone()), Ok(()));
    let hello = Hello {
        generation: 999,
        core_build: CoreBuildId::new("core-build-new".to_owned()),
        previous: PreviousCoreState::Crash,
    };
    let holder = HolderDescriptor::new(
        HolderBuildId::new("holder-build-old".to_owned()),
        HolderInstanceId::new("holder-instance-old".to_owned()),
    );
    assert_eq!(
        negotiate(&hello, &holder, &records),
        Err(AdoptionError::UnsupportedGeneration {
            offered: 999,
            supported: 1,
        })
    );
    assert_eq!(records.held_terminals(), vec![retained.clone()]);

    let stale = HeldTerminal {
        terminal_incarnation: TerminalIncarnation::new(3),
        ..retained.clone()
    };
    assert_eq!(
        records.retain(stale),
        Err(AdoptionError::StaleTarget { terminal_id })
    );
    assert_eq!(records.held_terminals(), vec![retained]);

    let mut unsupported_header =
        FrameHeader::encode(MessageKind::Hello, 0).expect("empty hello frame fits");
    unsupported_header[4..6].copy_from_slice(&999_u16.to_le_bytes());
    assert_eq!(
        parse_header(&unsupported_header),
        Err(FrameError::UnsupportedGeneration(999))
    );
    assert_eq!(parse_header(&[0, 1, 2]), Err(FrameError::MalformedHeader));

    let mut malformed_header =
        FrameHeader::encode(MessageKind::Hello, 0).expect("empty hello frame fits");
    malformed_header[7] = 1;
    assert_eq!(
        parse_header(&malformed_header),
        Err(FrameError::MalformedHeader)
    );
    assert_eq!(
        FrameHeader::encode(MessageKind::Output, 16 * 1024 * 1024 + 1),
        Err(FrameError::FrameTooLarge(16 * 1024 * 1024 + 1))
    );
}
