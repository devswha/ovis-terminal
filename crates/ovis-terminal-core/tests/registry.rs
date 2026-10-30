//! Behavior tests for identity resolution, durable allocation and lifecycle.

use std::error::Error;
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, IdentityReservation, LifecycleEvent, MachineId, MachineRuntime,
    ProcessGeneration, ProcessHandle, ProcessId, ProcessIdentity, ProcessPid, ProcessStartToken,
    RelaunchEvidence, ResourceName, ResourceTarget, RuntimeId, RuntimeIdentity, ServerBootId,
    SurvivalEvidence, TargetIndex, TargetRequest, TerminalGeneration, TerminalId,
    TerminalLifecycle, TerminalState,
};
use ovis_terminal_store::{Store, StoreOptions};
use uuid::Uuid;

type TestResult<T> = Result<T, Box<dyn Error>>;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "ovis-registry-{}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

fn sample_scope(machine: u64, runtime: u64) -> TestResult<MachineRuntime> {
    Ok(MachineRuntime::new(
        MachineId::new(Uuid::from_u128(u128::from(machine)))?,
        RuntimeId::new(NonZeroU64::new(runtime).ok_or("zero runtime fixture")?),
    ))
}

fn sample_host(machine: u64, host: u64) -> TestResult<HostIdentity> {
    Ok(HostIdentity::new(
        MachineId::new(Uuid::from_u128(u128::from(machine)))?,
        HostBootId::new(NonZeroU64::new(host).ok_or("zero host fixture")?),
    ))
}

fn sample_target(scope: MachineRuntime, terminal: u64, name: &str) -> TestResult<ResourceTarget> {
    Ok(ResourceTarget::new(
        scope,
        TerminalId::try_from(terminal)?,
        ResourceName::new(name)?,
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

#[tokio::test]
async fn happy() -> TestResult<()> {
    let first = sample_target(sample_scope(1, 1)?, 1, "worker")?;
    let second = sample_target(sample_scope(2, 1)?, 1, "worker")?;
    let index = TargetIndex::from_targets([first.clone(), second]);
    let ambiguous = index.resolve(TargetRequest::Name {
        machine: None,
        name: ResourceName::new("worker")?,
    });
    let error = ambiguous.unwrap_err();
    assert_eq!(error.kind(), "ambiguous_target");
    assert_eq!(error.candidates().map(<[_]>::len), Some(2));

    assert_eq!(index.resolve(TargetRequest::Id(first.key()))?, first);
    let opaque_key = first.key();
    let renamed = first.with_name(ResourceName::new("renamed")?);
    assert_eq!(renamed.key(), opaque_key);
    assert_eq!(renamed.terminal_id(), first.terminal_id());
    assert_eq!(renamed.runtime_id(), first.runtime_id());
    let after_rename = TargetIndex::from_targets([renamed.clone()]);
    assert_eq!(
        after_rename.resolve(TargetRequest::Id(opaque_key))?,
        renamed
    );

    let allocated = TerminalLifecycle::new(
        sample_runtime(sample_host(1, 1)?, 1, 1)?,
        first.terminal_id(),
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
    assert_eq!(core_restart.process(), original.process());

    let upgrade = core_restart.transition(LifecycleEvent::Upgrade {
        server_boot_id: ServerBootId::new(NonZeroU64::new(3).ok_or("zero boot fixture")?),
        survival: SurvivalEvidence::HolderConfirmed(process(
            1,
            1,
            process_handle(42, "linux:boot-1:42:100")?,
        )?),
    })?;
    assert_eq!(upgrade.runtime_id(), original.runtime_id());
    assert_eq!(upgrade.process(), original.process());

    let reboot = upgrade.transition(LifecycleEvent::HostReboot {
        identity: sample_runtime(sample_host(1, 2)?, 2, 4)?,
    })?;
    assert_eq!(reboot.state(), TerminalState::RestoredMetadata);
    assert_eq!(reboot.process(), None);
    assert_eq!(reboot.generation(), original.generation());
    let relaunched = reboot.transition(LifecycleEvent::Relaunch {
        evidence: RelaunchEvidence::SpawnConfirmed,
        process: process(2, 2, process_handle(42, "linux:boot-2:42:12")?)?,
    })?;
    assert_eq!(
        relaunched.generation(),
        original.generation().checked_next()?
    );
    assert_eq!(relaunched.runtime_id().get(), 2);

    let root = TestRoot::new();
    let store = Store::open(&root.0, StoreOptions::default())?;
    let reservation = IdentityReservation::new(&store);
    let machine_id = reservation
        .load_or_reserve_machine(terminal_op("reserve-machine"))
        .await?;
    let first_id = reservation
        .reserve::<TerminalId>(terminal_op("reserve-first"))
        .await?;
    let snapshot = store.snapshot()?;
    assert!(reservation.is_reserved(&snapshot, first_id)?);
    store.close()?;

    let reopened = Store::open(&root.0, StoreOptions::default())?;
    let restored_machine = IdentityReservation::new(&reopened)
        .load_or_reserve_machine(terminal_op("reload-machine"))
        .await?;
    assert_eq!(restored_machine, machine_id);
    let second_id = IdentityReservation::new(&reopened)
        .reserve::<TerminalId>(terminal_op("reserve-second"))
        .await?;
    assert_ne!(first_id, second_id);
    reopened.close()?;
    std::fs::remove_dir_all(&root.0)?;
    Ok(())
}

#[tokio::test]
async fn failure() -> TestResult<()> {
    let zero = TerminalId::try_from(0_u64).unwrap_err();
    assert_eq!(zero.kind(), "invalid_identity");
    assert_eq!(TerminalId::try_from(u64::MAX)?.get(), u64::MAX);
    assert_eq!(
        MachineId::new(Uuid::from_u128(0)).unwrap_err().kind(),
        "invalid_identity"
    );

    let wrong_family = "workspace:7".parse::<TerminalId>().unwrap_err();
    assert_eq!(wrong_family.kind(), "invalid_identity");
    assert!(serde_json::from_str::<TerminalId>("\"workspace:7\"").is_err());
    assert_eq!(
        ResourceName::new("bad/name").unwrap_err().kind(),
        "invalid_name"
    );
    assert_eq!(
        ProcessStartToken::new("").unwrap_err().kind(),
        "invalid_identity"
    );

    let target = sample_target(sample_scope(1, 1)?, 1, "same")?;
    let index = TargetIndex::from_targets([target.clone()]);
    assert_eq!(
        index.resolve(TargetRequest::Missing).unwrap_err().kind(),
        "needs_target"
    );
    assert_eq!(
        index
            .resolve(TargetRequest::Name {
                machine: None,
                name: ResourceName::new("missing")?,
            })
            .unwrap_err()
            .kind(),
        "not_found"
    );
    assert_eq!(
        index
            .resolve(TargetRequest::CallerBinding {
                target: target.key(),
                expected_scope: MachineRuntime::new(
                    target.machine_id(),
                    RuntimeId::new(NonZeroU64::new(2).ok_or("zero runtime fixture")?),
                ),
            })
            .unwrap_err()
            .kind(),
        "cross_lifetime"
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
    let previous = initial.process().ok_or("initial process was not adopted")?;
    let stale = replacement.resolve_process(previous).unwrap_err();
    assert_eq!(stale.kind(), "stale_target");
    assert_eq!(
        replacement.process(),
        Some(&process(2, 2, process_handle(42, "linux:boot-1:42:200")?)?)
    );
    Ok(())
}

fn terminal_op(value: &str) -> ovis_terminal_store::OpId {
    ovis_terminal_store::OpId::new(value)
}
