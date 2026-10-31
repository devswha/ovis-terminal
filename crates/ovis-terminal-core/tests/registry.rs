//! Behavior tests for identity resolution, durable allocation and lifecycle.

use std::error::Error;
use std::num::NonZeroU64;

use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, LifecycleEvent, MachineId, ProcessGeneration, ProcessHandle,
    ProcessId, ProcessIdentity, ProcessPid, ProcessStartToken, RelaunchEvidence, RuntimeId,
    RuntimeIdentity, ServerBootId, SurvivalEvidence, TargetIndex, TargetRequest,
    TerminalGeneration, TerminalId, TerminalLifecycle, TerminalState,
};
use uuid::Uuid;

type TestResult<T> = Result<T, Box<dyn Error>>;

fn sample_host(machine: u64, host: u64) -> TestResult<HostIdentity> {
    Ok(HostIdentity::new(
        MachineId::new(Uuid::from_u128(u128::from(machine)))?,
        HostBootId::new(NonZeroU64::new(host).ok_or("zero host fixture")?),
    ))
}

fn sample_runtime(host: HostIdentity, runtime: u64, boot: u64) -> TestResult<RuntimeIdentity> {
    Ok(RuntimeIdentity::new(
        host,
        RuntimeId::new(NonZeroU64::new(runtime).ok_or("zero runtime fixture")?),
        ServerBootId::new(NonZeroU64::new(boot).ok_or("zero boot fixture")?),
    ))
}

fn process_handle(pid: u64, token: &str) -> TestResult<ProcessHandle> {
    Ok(ProcessHandle::new(
        ProcessPid::try_from(pid)?,
        ProcessStartToken::new(token)?,
    ))
}

fn process(id: u64, generation: u64, handle: ProcessHandle) -> TestResult<ProcessIdentity> {
    Ok(ProcessIdentity::new(
        ProcessId::try_from(id)?,
        ProcessGeneration::new(generation)?,
        handle,
    ))
}

#[test]
fn happy() -> TestResult<()> {
    let allocated = TerminalLifecycle::new(
        sample_runtime(sample_host(1, 1)?, 1, 1)?,
        TerminalId::try_from(1_u64)?,
        TerminalGeneration::new(1)?,
    );
    let original = allocated.transition(LifecycleEvent::InitialSpawn {
        evidence: RelaunchEvidence::SpawnConfirmed,
        process: process(1, 1, process_handle(42, "linux:boot-1:42:100")?)?,
    })?;
    let core_restart = original.transition(LifecycleEvent::CoreRestart {
        server_boot_id: ServerBootId::new(NonZeroU64::new(2).ok_or("zero boot fixture")?),
        survival: SurvivalEvidence::HolderConfirmed(process(
            1,
            1,
            process_handle(42, "linux:boot-1:42:100")?,
        )?),
    })?;
    assert_eq!(core_restart.runtime_id(), original.runtime_id());
    assert_eq!(core_restart.terminal_id(), original.terminal_id());
    assert_eq!(core_restart.generation(), original.generation());
    assert_eq!(core_restart.process(), original.process());
    let reused_core_boot = core_restart.transition(LifecycleEvent::CoreRestart {
        server_boot_id: ServerBootId::new(NonZeroU64::new(2).ok_or("zero boot fixture")?),
        survival: SurvivalEvidence::HolderConfirmed(process(
            1,
            1,
            process_handle(42, "linux:boot-1:42:100")?,
        )?),
    });
    assert!(
        matches!(reused_core_boot, Err(error) if error.kind() == "invalid_transition"),
        "core restart must adopt the new server boot ID"
    );

    let upgrade = core_restart.transition(LifecycleEvent::Upgrade {
        server_boot_id: ServerBootId::new(NonZeroU64::new(3).ok_or("zero boot fixture")?),
        survival: SurvivalEvidence::HolderConfirmed(process(
            1,
            1,
            process_handle(42, "linux:boot-1:42:100")?,
        )?),
    })?;
    assert_eq!(upgrade.runtime_id(), original.runtime_id());
    assert_eq!(upgrade.terminal_id(), original.terminal_id());
    assert_eq!(upgrade.generation(), original.generation());
    assert_eq!(upgrade.process(), original.process());
    let reused_upgrade_boot = upgrade.transition(LifecycleEvent::Upgrade {
        server_boot_id: ServerBootId::new(NonZeroU64::new(3).ok_or("zero boot fixture")?),
        survival: SurvivalEvidence::HolderConfirmed(process(
            1,
            1,
            process_handle(42, "linux:boot-1:42:100")?,
        )?),
    });
    assert!(
        matches!(reused_upgrade_boot, Err(error) if error.kind() == "invalid_transition"),
        "upgrade must adopt the new server boot ID"
    );

    Ok(())
}

#[test]
fn host_reboot_and_relaunch_identity_domains() -> TestResult<()> {
    let before = TerminalLifecycle::new(
        sample_runtime(sample_host(1, 1)?, 1, 3)?,
        TerminalId::try_from(1_u64)?,
        TerminalGeneration::new(1)?,
    )
    .transition(LifecycleEvent::InitialSpawn {
        evidence: RelaunchEvidence::SpawnConfirmed,
        process: process(1, 1, process_handle(42, "linux:boot-1:42:100")?)?,
    })?;
    let reboot = before.transition(LifecycleEvent::HostReboot {
        identity: sample_runtime(sample_host(1, 2)?, 2, 4)?,
    })?;
    assert_eq!(reboot.state(), TerminalState::RestoredMetadata);
    assert_eq!(reboot.process(), None);
    assert_ne!(reboot.runtime_id(), before.runtime_id());
    assert_eq!(reboot.terminal_id(), before.terminal_id());
    assert_eq!(reboot.generation(), before.generation());
    let reused_boot = reboot.transition(LifecycleEvent::CoreRestart {
        server_boot_id: ServerBootId::new(NonZeroU64::new(4).ok_or("zero boot fixture")?),
        survival: SurvivalEvidence::NoLiveProcess,
    });
    assert!(
        matches!(reused_boot, Err(error) if error.kind() == "invalid_transition"),
        "host reboot must adopt the new server boot ID"
    );

    let relaunched = reboot.transition(LifecycleEvent::Relaunch {
        evidence: RelaunchEvidence::SpawnConfirmed,
        process: process(2, 2, process_handle(42, "linux:boot-2:42:12")?)?,
    })?;
    assert_eq!(relaunched.generation(), reboot.generation().checked_next()?);
    assert_eq!(relaunched.terminal_id(), reboot.terminal_id());
    assert_eq!(relaunched.runtime_id(), reboot.runtime_id());
    assert_eq!(
        relaunched.process().map(ProcessIdentity::generation),
        Some(ProcessGeneration::new(2)?)
    );
    let relaunched_process = relaunched
        .process()
        .ok_or("reboot relaunch has no process identity")?
        .clone();
    let reused_boot = relaunched.transition(LifecycleEvent::CoreRestart {
        server_boot_id: ServerBootId::new(NonZeroU64::new(4).ok_or("zero boot fixture")?),
        survival: SurvivalEvidence::HolderConfirmed(relaunched_process),
    });
    assert!(
        matches!(reused_boot, Err(error) if error.kind() == "invalid_transition"),
        "relaunch must preserve the adopted server boot ID"
    );
    Ok(())
}

#[test]
fn failure() -> TestResult<()> {
    assert_eq!(
        TargetIndex::default()
            .resolve(TargetRequest::Missing)
            .unwrap_err()
            .kind(),
        "needs_target"
    );

    let allocated = TerminalLifecycle::new(
        sample_runtime(sample_host(1, 1)?, 1, 1)?,
        TerminalId::new(NonZeroU64::new(1).ok_or("zero terminal fixture")?),
        TerminalGeneration::new(1)?,
    );
    let initial = allocated.transition(LifecycleEvent::InitialSpawn {
        evidence: RelaunchEvidence::SpawnConfirmed,
        process: process(1, 1, process_handle(42, "linux:boot-1:42:100")?)?,
    })?;
    let unproven = initial
        .transition(LifecycleEvent::CoreRestart {
            server_boot_id: ServerBootId::new(NonZeroU64::new(2).ok_or("zero boot fixture")?),
            survival: SurvivalEvidence::HolderConfirmed(process(
                1,
                1,
                process_handle(42, "linux:boot-1:42:stale")?,
            )?),
        })
        .unwrap_err();
    assert_eq!(unproven.kind(), "stale_target");
    let lost = initial.transition(LifecycleEvent::HolderLost)?;
    assert_eq!(lost.state(), TerminalState::Lost);
    assert_eq!(lost.runtime_id(), initial.runtime_id());
    assert_eq!(lost.terminal_id(), initial.terminal_id());
    assert_eq!(lost.generation(), initial.generation());
    assert_eq!(lost.process(), None);
    let reused_lost_boot = lost.transition(LifecycleEvent::CoreRestart {
        server_boot_id: ServerBootId::new(NonZeroU64::new(1).ok_or("zero boot fixture")?),
        survival: SurvivalEvidence::NoLiveProcess,
    });
    assert!(
        matches!(reused_lost_boot, Err(error) if error.kind() == "invalid_transition"),
        "holder loss must preserve the current server boot ID"
    );
    let reused_pid_without_new_start = lost
        .transition(LifecycleEvent::Relaunch {
            evidence: RelaunchEvidence::SpawnConfirmed,
            process: process(2, 2, process_handle(42, "linux:boot-1:42:100")?)?,
        })
        .unwrap_err();
    assert_eq!(reused_pid_without_new_start.kind(), "stale_target");
    let replacement = lost.transition(LifecycleEvent::Relaunch {
        evidence: RelaunchEvidence::SpawnConfirmed,
        process: process(2, 2, process_handle(42, "linux:boot-1:42:200")?)?,
    })?;
    assert_eq!(replacement.generation(), lost.generation().checked_next()?);
    assert_eq!(
        replacement.process().map(ProcessIdentity::generation),
        Some(ProcessGeneration::new(2)?)
    );
    let previous = initial.process().ok_or("initial process was not adopted")?;
    let stale = replacement.resolve_process(previous).unwrap_err();
    assert_eq!(stale.kind(), "stale_target");
    assert_eq!(
        replacement.process(),
        Some(&process(2, 2, process_handle(42, "linux:boot-1:42:200")?)?)
    );
    let replacement_process = replacement
        .process()
        .ok_or("holder-loss relaunch has no process identity")?
        .clone();
    let reused_boot_after_relaunch = replacement.transition(LifecycleEvent::CoreRestart {
        server_boot_id: ServerBootId::new(NonZeroU64::new(1).ok_or("zero boot fixture")?),
        survival: SurvivalEvidence::HolderConfirmed(replacement_process),
    });
    assert!(
        matches!(
            reused_boot_after_relaunch,
            Err(error) if error.kind() == "invalid_transition"
        ),
        "relaunch after holder loss must preserve the current server boot ID"
    );
    Ok(())
}
