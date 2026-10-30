//! Bounded model-based properties for adoption, exit replay, and acknowledgement.

use proptest::prelude::*;
use proptest::test_runner::{Config, FileFailurePersistence};

use super::*;

#[derive(Clone, Debug)]
enum Transition {
    Adopt,
    ReadoptSame,
    ReadoptStale,
    ReadoptWrongGeneration,
    Exit(u8),
    Ack(u8),
}

proptest! {
    #![proptest_config(Config {
        failure_persistence: Some(Box::new(FileFailurePersistence::WithSource("proptest-regressions"))),
        ..Config::default()
    })]

    #[test]
    fn generated_adoption_exit_ack_sequences_match_reference_model(
        terminal_name in "[a-z0-9-]{1,16}",
        transitions in prop::collection::vec(
            prop_oneof![
                Just(Transition::Adopt),
                Just(Transition::ReadoptSame),
                Just(Transition::ReadoptStale),
                Just(Transition::ReadoptWrongGeneration),
                (0_u8..=16).prop_map(Transition::Exit),
                (0_u8..=16).prop_map(Transition::Ack),
            ],
            1..32,
        ),
    ) {
        let terminal_id = TerminalId::new(terminal_name);
        let terminal_incarnation = TerminalIncarnation::new(1);
        let process_incarnation = ProcessIncarnation::new(2);
        let process_start_token = ProcessStartToken::new("process-start-2".to_owned());
        let holder = HolderDescriptor::new(
            HolderBuildId::new("holder-build".to_owned()),
            HolderInstanceId::new("holder-instance".to_owned()),
        );
        let supported = Hello {
            generation: 1,
            core_build: CoreBuildId::new("core-build".to_owned()),
            previous: PreviousCoreState::Crash,
        };
        let mut table = AdoptionTable::default();
        let mut record_present = false;
        let mut output_offset = 0_u64;
        let mut final_offset = None;

        for transition in transitions {
            match transition {
                Transition::Adopt => {
                    let target = TerminalTarget {
                        terminal_id: terminal_id.clone(),
                        terminal_incarnation,
                        process_incarnation: Some(process_incarnation),
                    };
                    let record = HeldTerminal {
                        terminal_id: terminal_id.clone(),
                        terminal_incarnation,
                        process_incarnation: Some(process_incarnation),
                        pid: Some(2),
                        start_token: Some(process_start_token.clone()),
                        output_offset,
                        input_sequence: 0,
                        exit: final_offset.map(|offset| Exit {
                            target,
                            code: Some(0),
                            signal: None,
                            exit_unknown: None,
                            io_failure: None,
                            final_offset: offset,
                        }),
                    };
                    prop_assert!(table.retain(record).is_ok());
                    record_present = true;
                }
                Transition::ReadoptSame => {
                    let welcome = negotiate(&supported, &holder, &table)
                        .expect("supported peer always re-adopts");
                    prop_assert_eq!(welcome.terminals.len(), usize::from(record_present));
                    prop_assert_eq!(
                        welcome
                            .terminals
                            .first()
                            .and_then(|record| record.exit.as_ref())
                            .map(|record| record.final_offset),
                        final_offset
                    );
                }
                Transition::ReadoptStale => {
                    let stale_target = TerminalTarget {
                        terminal_id: terminal_id.clone(),
                        terminal_incarnation: TerminalIncarnation::new(3),
                        process_incarnation: Some(process_incarnation),
                    };
                    let error = table
                        .acknowledge_exit(&stale_target, output_offset)
                        .expect_err("old incarnation cannot target retained terminal");
                    prop_assert_eq!(
                        error.kind(),
                        if record_present { "stale_target" } else { "exit_not_retained" }
                    );
                }
                Transition::ReadoptWrongGeneration => {
                    let wrong = Hello { generation: 999, ..supported.clone() };
                    let error = negotiate(&wrong, &holder, &table)
                        .expect_err("unsupported generation is rejected");
                    prop_assert_eq!(error.kind(), "unsupported_generation");
                }
                Transition::Exit(offset) => {
                    let next_final_offset = u64::from(offset);
                    let next_output_offset = output_offset.max(next_final_offset);
                    let target = TerminalTarget {
                        terminal_id: terminal_id.clone(),
                        terminal_incarnation,
                        process_incarnation: Some(process_incarnation),
                    };
                    let record = HeldTerminal {
                        terminal_id: terminal_id.clone(),
                        terminal_incarnation,
                        process_incarnation: Some(process_incarnation),
                        pid: Some(2),
                        start_token: Some(process_start_token.clone()),
                        output_offset: next_output_offset,
                        input_sequence: 0,
                        exit: Some(Exit {
                            target,
                            code: Some(0),
                            signal: None,
                            exit_unknown: None,
                            io_failure: None,
                            final_offset: next_final_offset,
                        }),
                    };
                    let result = table.retain(record);
                    if let Some(prior_final_offset) = final_offset {
                        if prior_final_offset == next_final_offset {
                            prop_assert!(result.is_ok());
                            output_offset = next_output_offset;
                        } else {
                            prop_assert_eq!(
                                result.expect_err("conflicting exit evidence is rejected").kind(),
                                "stale_evidence"
                            );
                        }
                    } else {
                        prop_assert!(result.is_ok());
                        record_present = true;
                        output_offset = next_output_offset;
                        final_offset = Some(next_final_offset);
                    }
                }
                Transition::Ack(offset) => {
                    let acknowledged = u64::from(offset);
                    let target = TerminalTarget {
                        terminal_id: terminal_id.clone(),
                        terminal_incarnation,
                        process_incarnation: Some(process_incarnation),
                    };
                    if let Some(final_offset) = final_offset {
                        if acknowledged < final_offset {
                        let error = table
                            .acknowledge_exit(&target, acknowledged)
                            .expect_err("early acknowledgement must retain the exit");
                        prop_assert_eq!(error.kind(), "exit_not_drained");
                        } else {
                        prop_assert!(table.acknowledge_exit(&target, acknowledged).is_ok());
                        prop_assert!(table.held_terminals().is_empty());
                        break;
                        }
                    } else {
                        let error = table
                            .acknowledge_exit(&target, acknowledged)
                            .expect_err("exit must exist before acknowledgement");
                        prop_assert_eq!(error.kind(), "exit_not_retained");
                    }
                }
            }
            let retained_records = table.held_terminals();
            prop_assert_eq!(retained_records.len(), usize::from(record_present));
            if record_present {
                let retained = retained_records.first().expect("model marks one record present");
                prop_assert_eq!(
                    retained.exit.as_ref().map(|exit| exit.final_offset),
                    final_offset
                );
                prop_assert_eq!(retained.output_offset, output_offset);
            }
        }
    }
}
