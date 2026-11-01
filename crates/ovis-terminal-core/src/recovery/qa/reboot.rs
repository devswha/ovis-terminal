//! Host reboot and inconclusive boot evidence: restored metadata or unknown, never a
//! surviving pre-reboot process, even when a holder still answers on the socket.

use ovis_terminal_protocol::domain::recovery::{IncidentKind, Survival, UnknownReason};
use ovis_terminal_protocol::domain::restart::identity::Cause;
use ovis_terminal_protocol::schema::scalar::TimestampMs;

use super::{Served, State, TestResult, running, until};
use crate::recovery::{Adopted, BootVerdict, GroupId, OsBootId, read_os_boot, recover};

#[test]
fn a_changed_boot_identity_lists_restored_metadata_and_adopts_nothing() -> TestResult {
    // Given: a recorded program and a boot stamp written under another boot.
    let state = State::new()?;
    let plan = state.plan()?;
    let served = Served::new(&state.0, GroupId::shared("session", "task-49"))?;
    let mut old = Adopted::adopt(&served.group, plan.core_build.clone(), plan.bound)?;
    let recorded = served.spawn("terminal:3", "printf 'ARMED\\r\\n'; read gate")?;
    until(&mut old, (&recorded.target, b"ARMED"))?;
    drop(old);
    plan.stamp()
        .record(&OsBootId::new("linux:a-previous-boot".to_owned()))?;

    // When: recovery runs.
    let recovered = recover(
        &plan,
        std::slice::from_ref(&served.group),
        std::slice::from_ref(&recorded),
    )?;

    // Then: metadata is restored, no process is claimed, no holder adopted.
    let report = &recovered.report;
    assert_eq!(report.cause, Some(Cause::HostReboot));
    let verdicts: Vec<_> = report.terminals.iter().map(|t| t.survival).collect();
    assert_eq!(verdicts, vec![Survival::RestoredMetadata {}]);
    assert!(report.terminals.iter().all(|t| t.screen.is_none()));
    assert!(recovered.links.is_empty());
    assert_eq!(report.evidence(TimestampMs::new(1)).restored, Vec::new());
    let kinds: Vec<_> = plan
        .incidents()
        .list()?
        .into_iter()
        .map(|i| i.kind)
        .collect();
    assert_eq!(kinds, vec![IncidentKind::HostReboot]);

    // And: the current boot is recorded only after recovery; the next crash compares to it.
    let current = read_os_boot()?;
    assert_eq!(plan.stamp().compare(Ok(&current))?, BootVerdict::Same);
    assert!(running(&recorded), "recovery signalled nothing");
    Ok(())
}

#[test]
fn without_recorded_boot_evidence_survival_is_unknown_and_nothing_is_adopted() -> TestResult {
    // Given: a live program but no boot stamp at all.
    let state = State::new()?;
    let plan = state.plan()?;
    let served = Served::new(&state.0, GroupId::shared("session", "task-49"))?;
    let mut old = Adopted::adopt(&served.group, plan.core_build.clone(), plan.bound)?;
    let recorded = served.spawn("terminal:4", "printf 'ARMED\\r\\n'; read gate")?;
    until(&mut old, (&recorded.target, b"ARMED"))?;
    drop(old);

    // When: recovery runs.
    let recovered = recover(
        &plan,
        std::slice::from_ref(&served.group),
        std::slice::from_ref(&recorded),
    )?;

    // Then: the verdict is unknown, no cause is claimed and nothing may relaunch it.
    let report = &recovered.report;
    assert_eq!(report.cause, None);
    let terminal = report.terminals.first().ok_or("verdict")?;
    let unknown = Survival::Unknown {
        reason: UnknownReason::BootEvidenceUnreadable,
    };
    assert_eq!(terminal.survival, unknown);
    assert!(!terminal.survival.may_relaunch());
    assert!(recovered.links.is_empty());
    let kinds: Vec<_> = plan
        .incidents()
        .list()?
        .into_iter()
        .map(|i| i.kind)
        .collect();
    assert_eq!(kinds, vec![IncidentKind::BootEvidenceUnreadable]);
    assert!(running(&recorded));
    Ok(())
}
