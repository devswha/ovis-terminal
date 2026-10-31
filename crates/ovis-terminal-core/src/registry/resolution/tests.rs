use std::error::Error;

use uuid::Uuid;

use crate::registry::{
    MachineId, MachineRuntime, ResourceName, ResourceTarget, RuntimeId, TargetIndex, TargetRequest,
    TerminalId,
};

type TestResult<T> = Result<T, Box<dyn Error>>;

fn target(machine: u64, name: &str) -> TestResult<ResourceTarget> {
    Ok(ResourceTarget::new(
        MachineRuntime::new(
            MachineId::new(Uuid::from_u128(u128::from(machine)))?,
            RuntimeId::try_from(1_u64)?,
        ),
        TerminalId::try_from(1_u64)?,
        ResourceName::new(name)?,
    ))
}

#[test]
fn target_resolution_returns_candidates_and_preserves_scoped_identity() -> TestResult<()> {
    let first = target(1, "worker")?;
    let second = target(2, "worker")?;
    let index = TargetIndex::from_targets([first.clone(), second.clone()]);
    let ambiguity = index.resolve(TargetRequest::Name {
        machine: None,
        name: ResourceName::new("worker")?,
    });
    let error = ambiguity.unwrap_err();
    assert_eq!(error.kind(), "ambiguous_target");
    let candidates = error.candidates().ok_or("ambiguity omitted candidates")?;
    let expected_candidates = [first.clone(), second];
    assert_eq!(candidates, expected_candidates.as_slice());
    assert_eq!(index.resolve(TargetRequest::Id(first.key()))?, first);

    let renamed = first.with_name(ResourceName::new("renamed")?);
    let renamed_index = TargetIndex::from_targets([renamed.clone()]);
    assert_eq!(renamed.key(), first.key());
    assert_eq!(
        renamed_index.resolve(TargetRequest::Id(first.key()))?,
        renamed
    );
    Ok(())
}

#[test]
fn caller_binding_rejects_a_different_runtime_lifetime() -> TestResult<()> {
    let target = target(1, "worker")?;
    let index = TargetIndex::from_targets([target.clone()]);
    let error = index
        .resolve(TargetRequest::CallerBinding {
            target: target.key(),
            expected_scope: MachineRuntime::new(target.machine_id(), RuntimeId::try_from(2_u64)?),
        })
        .unwrap_err();
    assert_eq!(error.kind(), "cross_lifetime");
    Ok(())
}
