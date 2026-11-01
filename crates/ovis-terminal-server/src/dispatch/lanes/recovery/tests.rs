use ovis_terminal_core::recovery::{Finding, Incidents};
use ovis_terminal_protocol::domain::recovery::{
    CrashRecovery, IncidentKind, LossReason, Survival, TerminalRecovery, UnknownReason,
};
use ovis_terminal_protocol::domain::restart::identity::Cause;
use ovis_terminal_protocol::schema::ids::{RuntimeLifetimeId, ServerBootId, TerminalId};
use ovis_terminal_protocol::schema::scalar::{Incarnation, TimestampMs};

use super::RecoveryLane;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const VERDICTS: [(&str, Survival); 5] = [
    ("terminal:1", Survival::Survived {}),
    ("terminal:2", Survival::RestoredMetadata {}),
    (
        "terminal:3",
        Survival::Lost {
            reason: LossReason::Gone,
        },
    ),
    (
        "terminal:4",
        Survival::Unknown {
            reason: UnknownReason::HolderUnreachable,
        },
    ),
    (
        "terminal:5",
        Survival::Relaunched {
            process_incarnation: Incarnation::new(9),
        },
    ),
];

fn lane(dir: &std::path::Path) -> std::sync::Arc<RecoveryLane> {
    let terminals = VERDICTS
        .iter()
        .map(|(id, survival)| {
            TerminalRecovery::new(
                (TerminalId::new((*id).to_owned()), Incarnation::new(1)),
                *survival,
                None,
            )
        })
        .collect();
    let report = CrashRecovery::new(
        Some(Cause::CoreCrash),
        (
            RuntimeLifetimeId::new("runtime:49".to_owned()),
            ServerBootId::new("server-boot:2".to_owned()),
        ),
        (terminals, Vec::new()),
    );
    RecoveryLane::new(report, Incidents::new(dir))
}

fn kind(result: Result<(), crate::dispatch::DispatchError>) -> Result<(), &'static str> {
    result.map_err(|error| error.kind())
}

#[test]
fn only_a_proven_loss_is_relaunchable_and_unknown_survival_refuses_input() {
    let dir = std::env::temp_dir();
    let lane = lane(&dir);
    let id = |n: &str| TerminalId::new(n.to_owned());
    let relaunch: Vec<_> = VERDICTS
        .iter()
        .map(|(n, _)| kind(lane.admit_relaunch(&id(n))))
        .collect();
    assert_eq!(
        relaunch,
        vec![
            Err("not_lost"),
            Ok(()),
            Ok(()),
            Err("recovering"),
            Err("not_lost")
        ]
    );
    let input: Vec<_> = VERDICTS
        .iter()
        .map(|(n, _)| kind(lane.admit_input(&id(n))))
        .collect();
    assert_eq!(
        input,
        vec![Ok(()), Ok(()), Ok(()), Err("recovering"), Ok(())]
    );
    assert_eq!(
        kind(lane.admit_relaunch(&id("terminal:99"))),
        Err("not_found")
    );
    assert_eq!(kind(lane.admit_input(&id("terminal:99"))), Ok(()));
}

#[test]
fn evidence_lists_only_survivors_and_relaunches_and_incident_reads_clear_nothing() -> TestResult {
    let dir = std::env::temp_dir().join(format!("ovis-t49-lane-{}", uuid::Uuid::now_v7()));
    let lane = lane(&dir);
    let restored: Vec<u64> = lane
        .evidence(TimestampMs::new(7))
        .restored
        .iter()
        .map(|incarnation| incarnation.get())
        .collect();
    assert_eq!(restored, vec![1, 9]);
    let boot = ServerBootId::new("server-boot:2".to_owned());
    Incidents::new(&dir).record(
        &boot,
        Finding::new(
            IncidentKind::HolderUnreachable,
            vec![TerminalId::new("terminal:4".to_owned())],
            None,
        ),
    )?;
    let first = lane.incidents()?;
    assert_eq!(first.len(), 1);
    assert_eq!(lane.incidents()?, first);
    std::fs::write(dir.join("incidents.jsonl"), b"not json\n")?;
    assert_eq!(
        lane.incidents().map(drop).map_err(|e| e.kind()),
        Err("journal_corrupt")
    );
    std::fs::remove_dir_all(dir)?;
    Ok(())
}
