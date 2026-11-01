//! Holder loss stays inside its group: an unreachable dedicated holder makes only its
//! terminal unknown, nothing is relaunched or signalled, and the shared group survives.

use ovis_terminal_protocol::domain::recovery::{IncidentKind, Survival, UnknownReason};
use ovis_terminal_protocol::schema::ids::TerminalId;

use super::{Served, State, TestResult, running, until};
use crate::recovery::{Adopted, GroupId, Placement, read_os_boot, recover};

#[test]
fn an_unreachable_dedicated_holder_is_unknown_without_relaunch_and_the_shared_group_survives()
-> TestResult {
    // Given: one program in the shared holder and one in a dedicated holder.
    let state = State::new()?;
    let plan = state.plan()?;
    let shared = Served::new(&state.0, GroupId::shared("session", "task-49"))?;
    let dedicated_id = ovis_terminal_holder::protocol::TerminalId::new("terminal:8".to_owned());
    let dedicated = Served::new(&state.0, GroupId::dedicated(&dedicated_id))?;
    assert_eq!(dedicated.group.id.placement(), Placement::Dedicated);
    let mut link = Adopted::adopt(&shared.group, plan.core_build.clone(), plan.bound)?;
    let mut other = Adopted::adopt(&dedicated.group, plan.core_build.clone(), plan.bound)?;
    let kept = shared.spawn("terminal:7", "printf 'ARMED\\r\\n'; read gate")?;
    let isolated = dedicated.spawn("terminal:8", "printf 'ARMED\\r\\n'; read gate")?;
    until(&mut link, (&kept.target, b"ARMED"))?;
    until(&mut other, (&isolated.target, b"ARMED"))?;
    plan.stamp().record(&read_os_boot()?)?;

    // When: the core crashes and the dedicated holder's socket stops answering.
    drop((link, other));
    dedicated.unplug()?;
    let recovered = recover(
        &plan,
        &[shared.group.clone(), dedicated.group.clone()],
        &[kept.clone(), isolated.clone()],
    )?;

    // Then: only the dedicated terminal is unknown, and it is not relaunchable.
    let verdicts: Vec<_> = recovered
        .report
        .terminals
        .iter()
        .map(|t| t.survival)
        .collect();
    let unknown = Survival::Unknown {
        reason: UnknownReason::HolderUnreachable,
    };
    assert_eq!(verdicts, vec![Survival::Survived {}, unknown]);
    assert!(!unknown.may_relaunch());
    assert_eq!(recovered.links.len(), 1);

    // And: the incident names only the unreachable group's terminal.
    let incidents = plan.incidents().list()?;
    let unreachable: Vec<_> = incidents
        .iter()
        .filter(|incident| incident.kind == IncidentKind::HolderUnreachable)
        .map(|incident| incident.terminals.clone())
        .collect();
    assert_eq!(
        unreachable,
        vec![vec![TerminalId::new("terminal:8".to_owned())]]
    );

    // And: no program was spawned twice or signalled: both still run as recorded.
    let shared_link = recovered.links.get(&shared.group.id).ok_or("shared link")?;
    assert_eq!(shared_link.welcome().terminals.len(), 1);
    assert!(running(&kept) && running(&isolated));
    Ok(())
}
