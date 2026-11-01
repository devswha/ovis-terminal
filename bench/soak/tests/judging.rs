//! The verdict, the fixture's pass rule and the tally, on hand-built facts.

use std::time::Duration;

use ovis_terminal_holder::protocol::{
    ProcessIncarnation, ProcessStartToken, TerminalId, TerminalIncarnation, TerminalTarget,
};
use ovis_terminal_protocol::domain::recovery::{
    OutputGap, Survival, TerminalRecovery, UnknownReason,
};
use ovis_terminal_protocol::schema::ids;
use ovis_terminal_protocol::schema::scalar::{ByteOffset, Count, Incarnation};

use super::super::account::Tally;
use super::super::fixture::FixtureOutcome;
use super::super::observer::Continuity;
use super::super::plan::{Kind, Limits};
use super::super::verdict::{Measured, Probe, judge};
use super::super::wire::{BootMode, Event, Identity, Line, Ready, Recovery};

const MIB: u64 = 1 << 20;
const LIMITS: Limits = Limits {
    rss_bytes: 1024 * MIB,
    disk_bytes: 4096 * MIB,
    control_ms: 1000,
    queue_bytes: 256 * 1024,
    growth_bytes: 16 * MIB,
    growth_percent: 1,
};

fn measured(tally: &Tally) -> Measured<'_> {
    let counts = vec![(Kind::CoreCrash, 2), (Kind::PlannedRestart, 1)];
    Measured {
        duration: Duration::from_secs(1800),
        elapsed: Duration::from_millis(1_800_004),
        requested: counts.clone(),
        done: counts,
        tally,
        continuity: Continuity::default(),
        limits: LIMITS,
        rss_peak: 300 * MIB,
        rss_baseline: Some(200 * MIB),
        rss_final: Some(210 * MIB),
        disk_peak: 50 * MIB,
        planned_exits: vec![Some(0)],
        probe: Some(Probe {
            dedicated: "unknown:holder_unreachable".to_owned(),
            shared_survived: 49,
            shared_expected: 49,
            shared_identities_unchanged: true,
        }),
    }
}

fn rules(measured: &Measured<'_>) -> Vec<&'static str> {
    judge(measured)
        .iter()
        .map(|violation| violation.rule)
        .collect()
}

fn answered() -> Tally {
    let mut tally = Tally::default();
    tally.pong(12);
    tally
}

#[test]
fn a_complete_clean_run_passes_and_each_broken_fact_fails_its_own_rule() {
    let tally = answered();
    assert_eq!(rules(&measured(&tally)), Vec::<&str>::new());

    let mut short = measured(&tally);
    short.elapsed = Duration::from_millis(1_799_999);
    assert_eq!(rules(&short), vec!["incomplete_duration"]);

    let mut skipped = measured(&tally);
    skipped.done = vec![(Kind::CoreCrash, 1), (Kind::PlannedRestart, 1)];
    assert_eq!(rules(&skipped), vec!["disruption_count"]);

    // 200 MiB baseline allows 16 MiB + 2 MiB; 18 MiB + 1 byte is too much.
    let mut grown = measured(&tally);
    grown.rss_final = Some(218 * MIB + 1);
    assert_eq!(rules(&grown), vec!["rss_growth"]);
    grown.rss_final = Some(218 * MIB);
    assert_eq!(rules(&grown), Vec::<&str>::new());

    let mut heavy = measured(&tally);
    heavy.rss_peak = 1024 * MIB + 1;
    assert_eq!(rules(&heavy), vec!["rss_limit"]);

    let mut crashed_handover = measured(&tally);
    crashed_handover.planned_exits = vec![Some(1)];
    assert_eq!(rules(&crashed_handover), vec!["planned_restart_exit"]);
}

#[test]
fn the_probe_must_lose_only_the_dedicated_terminal() {
    let tally = answered();
    let mut wrong = measured(&tally);
    wrong.probe = Some(Probe {
        dedicated: "survived".to_owned(),
        shared_survived: 48,
        shared_expected: 49,
        shared_identities_unchanged: true,
    });
    assert_eq!(
        rules(&wrong),
        vec!["dedicated_holder_loss", "shared_holder_isolation"]
    );
}

#[test]
fn late_or_missing_control_replies_and_queue_overruns_fail() {
    let mut late = answered();
    late.late();
    assert_eq!(rules(&measured(&late)), vec!["control_unavailable"]);
    let silent = Tally::default();
    assert_eq!(rules(&measured(&silent)), vec!["control_unprobed"]);
    let mut overrun = answered();
    overrun.max_queued_bytes = LIMITS.queue_bytes + 1;
    assert_eq!(rules(&measured(&overrun)), vec!["viewer_queue_bound"]);
}

#[test]
fn the_fixture_passes_only_without_exact_recovery_and_without_a_resend() {
    let honest = FixtureOutcome {
        screen_exact: Some(false),
        gaps: vec![OutputGap::new(ByteOffset::new(1244), ByteOffset::new(9000))],
        outcome: "outcome_unknown".to_owned(),
        ..FixtureOutcome::default()
    };
    assert!(honest.passed());
    let exact = FixtureOutcome {
        screen_exact: Some(true),
        ..honest.clone()
    };
    let gapless = FixtureOutcome {
        gaps: Vec::new(),
        ..honest.clone()
    };
    let known = FixtureOutcome {
        outcome: "committed".to_owned(),
        ..honest.clone()
    };
    let resent = FixtureOutcome {
        resent: true,
        ..honest.clone()
    };
    let unreported = FixtureOutcome {
        screen_exact: None,
        ..honest
    };
    for broken in [exact, gapless, known, resent, unreported] {
        assert!(!broken.passed(), "{broken:?}");
    }
}

fn identity(index: u64, pid: u32) -> Identity {
    Identity {
        index: Count::new(index),
        target: TerminalTarget {
            terminal_id: TerminalId::new(format!("terminal:soak-{index}")),
            terminal_incarnation: TerminalIncarnation::new(1),
            process_incarnation: Some(ProcessIncarnation::new(1)),
        },
        group: Count::new(0),
        pid: ids::ProcessPid::new(pid),
        start_token: ProcessStartToken::new(format!("linux:boot:{pid}")),
        input_sequence: None,
    }
}

fn ready(mode: BootMode, (pids, survival): (&[u32], Survival)) -> Ready {
    let terminals = (0_u64..)
        .zip(pids)
        .map(|(index, pid)| identity(index, *pid))
        .collect();
    let recovered = (0_u64..)
        .zip(pids)
        .map(|(index, _)| {
            let id = ids::TerminalId::new(format!("terminal:soak-{index}"));
            TerminalRecovery::new((id, Incarnation::new(1)), survival, None)
        })
        .collect();
    Ready {
        mode,
        boot: Count::new(1),
        terminals,
        recovery: (mode != BootMode::Fresh).then_some(Recovery::Handover {
            terminals: recovered,
        }),
        interrupted: Vec::new(),
    }
}

#[test]
fn a_survivor_under_another_pid_or_an_unexcused_loss_is_counted() {
    let mut tally = Tally::default();
    tally.ready(&ready(
        BootMode::Fresh,
        (&[100, 101], Survival::Survived {}),
    ));
    tally.ready(&ready(
        BootMode::Handover,
        (&[100, 999], Survival::Survived {}),
    ));
    assert_eq!(tally.identity_changes, 1);
    assert_eq!(tally.losses.len(), 1);
    // Both survivors arrived without a screen record.
    assert_eq!(
        tally.recoveries.first().map(|note| note.unreported_screens),
        Some(2)
    );

    let unknown = Survival::Unknown {
        reason: UnknownReason::HolderUnreachable,
    };
    let mut probed = Tally::default();
    probed.ready(&ready(
        BootMode::Fresh,
        (&[100, 101], Survival::Survived {}),
    ));
    probed.excused = Some(1);
    probed.ready(&ready(BootMode::Crash, (&[100, 101], unknown)));
    assert_eq!(
        probed
            .losses
            .iter()
            .map(|loss| loss.terminal)
            .collect::<Vec<_>>(),
        vec![0]
    );
}

#[test]
fn an_output_offset_that_moves_backwards_is_a_regression() {
    let mut tally = Tally::default();
    tally.ready(&ready(BootMode::Fresh, (&[100], Survival::Survived {})));
    for head in [10, 20, 15, 30] {
        let event = Event::Progress {
            offsets: vec![Some(ByteOffset::new(head))],
            checkpoints: Count::new(0),
        };
        tally.line(
            1,
            &Line::Event {
                seq: ovis_terminal_protocol::schema::scalar::EventSeq::new(0),
                event,
            },
        );
    }
    assert_eq!(tally.offset_regressions, 1);
    assert_eq!(tally.last_offsets, vec![Some(30)]);
}
