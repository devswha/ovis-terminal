use ovis_terminal_protocol::domain::health::RecoveryPhase;
use ovis_terminal_protocol::domain::recovery::{
    CrashRecovery, IncidentKind, Survival, TerminalRecovery, UnknownReason,
};
use ovis_terminal_protocol::schema::ids::{RuntimeLifetimeId, ServerBootId, TerminalId};
use ovis_terminal_protocol::schema::scalar::{Incarnation, OpenEnum, TimestampMs};
use ovis_terminal_store::{Store, StoreOptions, read_quarantine_evidence};

use super::{StateDir, health, metrics, runtime};
use crate::health::{Health, RecoveryLedger};
use crate::recovery::{Finding, Incidents};

fn boot() -> ServerBootId {
    ServerBootId::new(runtime().server_boot_id().to_string())
}

#[test]
fn a_recorded_corruption_survives_every_read_and_a_reopen() {
    let dir = StateDir::new();
    let health = health(&dir);
    assert_eq!(
        metrics(&health).recovery_state.phase,
        OpenEnum::Known(RecoveryPhase::Clean)
    );
    let finding = Finding::new(IncidentKind::StoreCorrupted, Vec::new(), None);
    health.recovery().record(finding).expect("recorded");
    let first = metrics(&health).recovery_state;
    assert!(first.corrupted);
    assert_eq!(first.phase, OpenEnum::Known(RecoveryPhase::Degraded));
    assert_eq!(first.incidents.get(), 1);
    assert_eq!(
        metrics(&health).recovery_state,
        first,
        "reading acknowledges nothing"
    );
    assert_eq!(health.status(&[]).recovery_state, Some(first.clone()));
    let reopened =
        RecoveryLedger::open(Incidents::new(&dir.0), (boot(), TimestampMs::new(2)), None)
            .expect("reopened");
    assert_eq!(
        reopened.state(&[]),
        first,
        "the journal, not memory, holds the fact"
    );
}

#[test]
fn an_undecodable_journal_is_reported_corrupted_and_left_as_it_is() {
    let dir = StateDir::new();
    let journal = dir.0.join("incidents.jsonl");
    std::fs::write(&journal, b"{not an incident}\n").expect("damaged journal");
    let ledger = RecoveryLedger::open(Incidents::new(&dir.0), (boot(), TimestampMs::new(1)), None)
        .expect("opened");
    assert!(ledger.state(&[]).corrupted);
    assert_eq!(
        std::fs::read(&journal).expect("still there"),
        b"{not an incident}\n"
    );
}

#[test]
fn quarantined_store_material_is_corruption() {
    let dir = StateDir::new();
    let db = dir.0.join("state.db");
    std::fs::write(&db, b"this is not a sqlite database").expect("damaged database");
    let refused = Store::open(&dir.0, StoreOptions::default()).expect_err("damaged material");
    assert_eq!(refused.kind(), "store_corrupted");
    assert!(!db.exists(), "the damaged database was moved to quarantine");
    let quarantined = read_quarantine_evidence(&dir.0).expect("evidence");
    assert_eq!(quarantined.len(), 1);
    let state = health(&dir).recovery().state(&quarantined);
    assert!(state.corrupted);
    assert_eq!(state.incidents.get(), 1);
}

#[test]
fn unknown_survival_after_a_crash_is_unsettled_until_proven() {
    let dir = StateDir::new();
    let unknown = TerminalRecovery::new(
        (TerminalId::new("t1".to_owned()), Incarnation::new(1)),
        Survival::Unknown {
            reason: UnknownReason::HolderUnreachable,
        },
        None,
    );
    let report = CrashRecovery::new(
        None,
        (RuntimeLifetimeId::new("59".to_owned()), boot()),
        (vec![unknown], Vec::new()),
    );
    let ledger = RecoveryLedger::open(
        Incidents::new(&dir.0),
        (boot(), TimestampMs::new(5)),
        Some(report),
    )
    .expect("opened");
    let health = Health::new(runtime(), ledger);
    let read = metrics(&health);
    assert_eq!(read.recovery_state.unsettled.get(), 1);
    assert_eq!(
        read.recovery_state.phase,
        OpenEnum::Known(RecoveryPhase::Degraded)
    );
    assert_eq!(read.recovery.len(), 1);
    let recorded = read.recovery.first().map(|evidence| evidence.recorded_ms);
    assert_eq!(recorded, Some(TimestampMs::new(5)));
}
