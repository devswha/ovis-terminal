//! One unplanned boot: boot evidence, holder-group adoption, survival verdicts,
//! screen rebuilds and durable incidents, in that order.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use ovis_terminal_holder::process::{ProcessId, StartTokenError, read_start_token};
use ovis_terminal_holder::protocol::{CoreBuildId, TerminalTarget};
use ovis_terminal_protocol::domain::recovery::{
    CrashRecovery, IncidentKind, Survival, TerminalRecovery,
};
use ovis_terminal_protocol::domain::restart::identity::Cause;
use ovis_terminal_protocol::schema::ids::{RuntimeLifetimeId, ServerBootId, TerminalId};
use ovis_terminal_protocol::schema::scalar::Incarnation;
use ovis_terminal_vt::VtConfig;

use super::adopt::Adopted;
use super::groups::{GroupId, HolderGroup};
use super::journal::{Finding, Incidents};
use super::replay::{Restored, Retained, restore};
use super::verdict::{Evidence, HolderAnswer, Liveness, Recorded, verdict};
use super::{BootStamp, BootVerdict, CheckpointStore, RecoveryError, read_os_boot};

/// Where recovery keeps its state and which identity it recovers under.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RecoveryPlan {
    /// State directory; recovery uses its `recovery/` child.
    pub state: PathBuf,
    /// Runtime lifetime served after recovery.
    pub runtime: RuntimeLifetimeId,
    /// Boot identity performing the recovery.
    pub boot: ServerBootId,
    /// Engine configuration of a screen rebuilt without a checkpoint.
    pub config: VtConfig,
    /// Build the recovering core adopts holders as.
    pub core_build: CoreBuildId,
    /// Longest wait for one holder message.
    pub bound: Duration,
}

impl RecoveryPlan {
    /// Recovers `state` as `boot` of `runtime`, rebuilding empty screens with `config`.
    #[must_use]
    pub fn new(
        state: PathBuf,
        (runtime, boot): (RuntimeLifetimeId, ServerBootId),
        config: VtConfig,
    ) -> Self {
        Self {
            state,
            runtime,
            boot,
            config,
            core_build: CoreBuildId::new(env!("CARGO_PKG_VERSION").to_owned()),
            bound: Duration::from_secs(15),
        }
    }

    /// The checkpoint store recovery reads; the live core writes the same one.
    #[must_use]
    pub fn checkpoints(&self) -> CheckpointStore {
        CheckpointStore::new(&self.state.join("recovery").join("checkpoints"))
    }

    /// The incident journal.
    #[must_use]
    pub fn incidents(&self) -> Incidents {
        Incidents::new(&self.state.join("recovery"))
    }

    /// The boot stamp.
    #[must_use]
    pub fn stamp(&self) -> BootStamp {
        BootStamp::new(&self.state.join("recovery"))
    }
}

/// What recovery produced: the report, rebuilt screens of survivors and adopted groups.
#[derive(Debug)]
#[non_exhaustive]
pub struct Recovered {
    /// The published report.
    pub report: CrashRecovery,
    /// Rebuilt screens of survivors.
    pub screens: Vec<(TerminalTarget, Restored)>,
    /// Groups that answered, with their links.
    pub links: BTreeMap<GroupId, Adopted>,
}

fn wire(target: &TerminalTarget) -> TerminalId {
    TerminalId::new(target.terminal_id.as_str().to_owned())
}

fn probe(pid: u32) -> Liveness {
    match read_start_token(ProcessId::new(pid)) {
        Ok(token) => Liveness::Running(token),
        Err(StartTokenError::ProcessGone { .. }) => Liveness::Gone,
        Err(_) => Liveness::Unreadable,
    }
}

/// Recovers `recorded` from `groups` under `plan`; never relaunches a program.
///
/// # Errors
///
/// Persistence failures of the journal, stamp or checkpoints, and holder link failures
/// after a group answered. A group that does not answer is not an error: its terminals
/// are reported unknown and an incident names them.
pub fn recover(
    plan: &RecoveryPlan,
    groups: &[HolderGroup],
    recorded: &[Recorded],
) -> Result<Recovered, RecoveryError> {
    let journal = plan.incidents();
    let current = read_os_boot();
    let boot = plan.stamp().compare(current.as_ref())?;
    let mut noted = Vec::new();
    let mut note = |finding| -> Result<(), RecoveryError> {
        noted.push(journal.record(&plan.boot, finding)?.incident_id);
        Ok(())
    };
    let everyone: Vec<TerminalId> = recorded.iter().map(|r| wire(&r.target)).collect();
    let (kind, cause) = match boot {
        BootVerdict::Same => (IncidentKind::CoreCrash, Some(Cause::CoreCrash)),
        BootVerdict::Rebooted => (IncidentKind::HostReboot, Some(Cause::HostReboot)),
        BootVerdict::Inconclusive => (IncidentKind::BootEvidenceUnreadable, None),
    };
    note(Finding::new(kind, everyone, None))?;
    let mut links = BTreeMap::new();
    for group in groups.iter().filter(|_| boot == BootVerdict::Same) {
        if let Ok(adopted) = Adopted::adopt(group, plan.core_build.clone(), plan.bound) {
            links.insert(group.id.clone(), adopted);
        } else {
            let lost = recorded.iter().filter(|r| r.group == group.id);
            note(Finding::new(
                IncidentKind::HolderUnreachable,
                lost.map(|r| wire(&r.target)).collect(),
                None,
            ))?;
        }
    }
    let verdicts: Vec<Survival> = recorded
        .iter()
        .map(|r| {
            let holder = links
                .get(&r.group)
                .map_or(HolderAnswer::Unreachable, |link| {
                    let held = link.welcome().terminals.iter();
                    HolderAnswer::Held(
                        held.into_iter()
                            .find(|h| h.terminal_id == r.target.terminal_id),
                    )
                });
            verdict(Evidence { boot, holder }, r, probe)
        })
        .collect();
    let mut terminals = Vec::with_capacity(recorded.len());
    let mut screens = Vec::new();
    for (r, survival) in recorded.iter().zip(verdicts) {
        let screen = match (survival, links.get_mut(&r.group)) {
            (Survival::Survived {}, Some(link)) => {
                let restored = rebuild(plan, (link, r), &mut note)?;
                let screen = restored.screen.clone();
                screens.push((r.target.clone(), restored));
                Some(screen)
            }
            _ => None,
        };
        let incarnation = Incarnation::new(r.target.terminal_incarnation.get());
        terminals.push(TerminalRecovery::new(
            (wire(&r.target), incarnation),
            survival,
            screen,
        ));
    }
    if let Ok(current) = &current {
        plan.stamp().record(current)?;
    }
    let report = CrashRecovery::new(
        cause,
        (plan.runtime.clone(), plan.boot.clone()),
        (terminals, noted),
    );
    Ok(Recovered {
        report,
        screens,
        links,
    })
}

/// Rebuilds one survivor's screen; a damaged or unsupported checkpoint is kept in
/// quarantine, noted, and the screen is rebuilt from retained output alone.
fn rebuild(
    plan: &RecoveryPlan,
    (link, recorded): (&mut Adopted, &Recorded),
    note: &mut impl FnMut(Finding) -> Result<(), RecoveryError>,
) -> Result<Restored, RecoveryError> {
    let checkpoint = match plan.checkpoints().load(&recorded.target) {
        Ok(checkpoint) => checkpoint,
        Err(
            ref error @ (RecoveryError::CheckpointCorrupted { ref quarantine }
            | RecoveryError::CheckpointUnsupported { ref quarantine }),
        ) => {
            let kind = match error {
                RecoveryError::CheckpointUnsupported { .. } => IncidentKind::CheckpointUnsupported,
                _ => IncidentKind::CheckpointCorrupted,
            };
            note(Finding::new(
                kind,
                vec![wire(&recorded.target)],
                Some(quarantine.clone()),
            ))?;
            None
        }
        Err(error) => return Err(error),
    };
    let from = checkpoint
        .as_ref()
        .map_or(0, |checkpoint| checkpoint.offset().get());
    let tail = link.tail(&recorded.target, from)?;
    let retained = Retained::new(tail.from_offset, tail.bytes);
    restore(
        (checkpoint.as_ref(), plan.config),
        &retained,
        recorded.fence,
    )
}
