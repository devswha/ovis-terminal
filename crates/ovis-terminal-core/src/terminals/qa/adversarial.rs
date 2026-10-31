//! Staged stopping, deduplicated recovery and control handoff on actual processes.

use ovis_terminal_core::registry::ResourceName;
use ovis_terminal_core::terminals::{
    ControlReceipt, ControlState, Field, KillRequest, KillStage, LabelRequest,
};
use ovis_terminal_holder::process::{ProcessId, read_start_token};
use ovis_terminal_holder::protocol::{Input, WriteGrant};

use super::*;

#[test]
fn repeated_kill_and_recover_never_retarget_the_replacement() -> TestResult {
    // Given: a kept program that explicitly ignores HUP and TERM before announcing readiness.
    let mut fixture = Fixture::new()?;
    let mut launch = fixture.launch(vec![
        "/bin/sh".into(),
        "-c".into(),
        "trap '' HUP TERM; printf ARMED; read gate".into(),
    ])?;
    launch.keep = true;
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let admitted = runtime.run(call(&fixture.session), launch)?;
    let original = runtime.await_spawn(admitted.id(), WAIT)?;
    let process = original.lifecycle.process().ok_or("process")?.clone();
    runtime.bind_agent(
        call(&fixture.session),
        (original.id(), process.handle().clone()),
    )?;
    fixture.marker(original.id(), b"ARMED")?;
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let kill_op = OpId::new(Uuid::now_v7().to_string());
    let request = KillRequest::immediate(original.id(), process.clone());

    // When: stopping escalates, then both stop and recovery requests are repeated.
    let killed = runtime.kill(Call::new(&fixture.session, kill_op.clone()), &request)?;
    assert_eq!(
        killed.delivered,
        vec![KillStage::Hangup, KillStage::Terminate, KillStage::Kill]
    );
    assert_eq!(killed.exit.as_ref().and_then(|exit| exit.signal), Some(9));
    assert_eq!(
        runtime
            .check_agent(&fixture.session, (original.id(), process.handle()))
            .expect_err("ended agent")
            .kind(),
        "process_replaced"
    );
    let recover_op = OpId::new(Uuid::now_v7().to_string());
    runtime.recover(
        Call::new(&fixture.session, recover_op.clone()),
        original.id(),
    )?;
    runtime.recover(
        Call::new(&fixture.session, recover_op.clone()),
        original.id(),
    )?;
    let replacement = runtime.await_spawn(original.id(), WAIT)?;
    let replay = runtime.kill(Call::new(&fixture.session, kill_op), &request)?;
    let recovered_again =
        runtime.recover(Call::new(&fixture.session, recover_op), original.id())?;

    // Then: one new incarnation exists; the repeated kill retains its old receipt.
    assert_eq!(replay, killed);
    assert_eq!(replacement.id(), original.id());
    assert_eq!(
        replacement.lifecycle.generation(),
        original.lifecycle.generation().checked_next()?
    );
    let next = replacement.lifecycle.process().ok_or("replacement")?;
    assert_eq!(next.generation(), process.generation().checked_next()?);
    assert_ne!(next, &process);
    assert_eq!(recovered_again.lifecycle.process(), Some(next));
    assert_eq!(replacement.agent_owner, None);
    assert_eq!(
        read_start_token(ProcessId::new(u32::try_from(next.pid().get())?))?.as_str(),
        next.start_token().as_str()
    );
    Ok(())
}

#[test]
fn metadata_removal_sends_no_signal_to_a_live_sibling() -> TestResult {
    // Given: an ended terminal and a sibling that reports every ordinary termination signal.
    let mut fixture = Fixture::new()?;
    let done = fixture.launch(vec!["/bin/sh".into(), "-c".into(), "exit 0".into()])?;
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let done = runtime.run(call(&fixture.session), done)?;
    let done = fixture.finish(done.id())?;
    let live = fixture.launch(vec!["/bin/sh".into(), "-c".into(), "stty -echo; trap 'printf SIGNAL' HUP TERM INT; printf ARMED; while IFS= read -r line; do printf BARRIER; done".into()])?;
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let live = runtime.run(call(&fixture.session), live)?;
    let live = runtime.await_spawn(live.id(), WAIT)?;
    fixture.marker(live.id(), b"ARMED")?;

    // When: only finished metadata is removed, then the sibling crosses an input barrier.
    let receipt = fixture
        .runtime
        .as_mut()
        .ok_or("runtime")?
        .rm(call(&fixture.session), done.id())?;
    let target = live.holder_target.clone().ok_or("holder target")?;
    fixture.holder.input(Input {
        grant: WriteGrant {
            grant_epoch: 1,
            fence_epoch: 1,
            process_incarnation: target.process_incarnation.ok_or("process incarnation")?,
        },
        target,
        sequence: 1,
        bytes: b"gate\n".to_vec(),
    })?;
    let bytes = fixture.marker(live.id(), b"BARRIER")?;

    // Then: the explicit receipt names the ended incarnation and no signal handler ran.
    assert_eq!(
        receipt.affected,
        done.lifecycle
            .last_process()
            .cloned()
            .into_iter()
            .collect::<Vec<_>>()
    );
    assert_eq!(bytes, b"ARMEDBARRIER");
    Ok(())
}

#[test]
fn control_restart_preserves_the_process_and_metadata_operations_are_independent() -> TestResult {
    // Given: an actual process with explicit metadata and native process inspection.
    let mut fixture = Fixture::new()?;
    let launch = fixture.launch(vec![
        "/bin/sh".into(),
        "-c".into(),
        "printf ARMED; read gate".into(),
    ])?;
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let created = runtime.run(call(&fixture.session), launch)?;
    let original = runtime.await_spawn(created.id(), WAIT)?;
    fixture.marker(original.id(), b"ARMED")?;
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    runtime.rename(
        call(&fixture.session),
        (original.id(), Some(ResourceName::new("renamed")?)),
    )?;
    runtime.label(
        call(&fixture.session),
        LabelRequest::new(
            original.id(),
            [("a".into(), "1".into()), ("b".into(), "2".into())].into(),
            Vec::new(),
        ),
    )?;
    runtime.label(
        call(&fixture.session),
        LabelRequest::new(
            original.id(),
            [("c".into(), "3".into())].into(),
            vec!["a".into()],
        ),
    )?;
    let info = runtime.process_info(&fixture.session, original.id())?;
    let Field::Known(launcher) = info.launcher else {
        return Err("launcher evidence unavailable".into());
    };
    assert_eq!(
        launcher.process.start_token(),
        original.lifecycle.process().ok_or("process")?.start_token()
    );

    // When: the old control parks for restart and a new boot adopts the same holder.
    let receipt = runtime.prepare_restart(call(&fixture.session))?;
    let ControlReceipt::HandoverPrepared { preserved, pending } = receipt else {
        return Err("wrong control scope".into());
    };
    assert_eq!(
        preserved,
        vec![original.lifecycle.process().ok_or("process")?.clone()]
    );
    assert_eq!(pending, Vec::new());
    fixture.reopen(2)?;

    // Then: identity and process survive, while control boot and ownership change explicitly.
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let adopted = runtime.get(&fixture.session, original.id())?;
    assert_eq!(adopted.lifecycle.process(), original.lifecycle.process());
    assert_eq!(
        adopted.lifecycle.generation(),
        original.lifecycle.generation()
    );
    assert_eq!(
        adopted.lifecycle.identity().server_boot_id(),
        ServerBootId::try_from(2)?
    );
    assert_eq!(adopted.control, ControlState::Running);
    assert_eq!(adopted.launch.name, Some(ResourceName::new("renamed")?));
    assert_eq!(
        adopted.launch.labels,
        [("b".into(), "2".into()), ("c".into(), "3".into())].into()
    );
    let receipt = runtime.control_stop(call(&fixture.session))?;
    assert!(
        matches!(receipt, ControlReceipt::Stopped { preserved, .. } if preserved == vec![original.lifecycle.process().ok_or("process")?.clone()])
    );
    Ok(())
}

#[test]
fn restored_metadata_never_inherits_live_process_or_agent_authority() -> TestResult {
    // Given: a real live record and independently supplied identities for a later host boot.
    let mut fixture = Fixture::new()?;
    let launch = fixture.launch(vec!["/bin/sh".into(), "-c".into(), "read gate".into()])?;
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let admitted = runtime.run(call(&fixture.session), launch)?;
    let original = runtime.await_spawn(admitted.id(), WAIT)?;
    let identity = RuntimeIdentity::new(
        HostIdentity::new(fixture.identity.machine_id(), HostBootId::try_from(2)?),
        RuntimeId::try_from(2)?,
        ServerBootId::try_from(2)?,
    );

    // When: the metadata is restored using the registry's reboot transition.
    let restored = original.clone().restore_metadata(identity)?;

    // Then: the last process is historical only. This test does not simulate a native reboot.
    assert_eq!(restored.id(), original.id());
    assert_eq!(restored.lifecycle.process(), None);
    assert_eq!(
        restored.lifecycle.last_process(),
        original.lifecycle.process()
    );
    assert_eq!(restored.agent_owner, None);
    assert_eq!(restored.holder_target, None);
    assert_eq!(
        restored.origin,
        ovis_terminal_protocol::schema::records::TerminalOrigin::RestoredMetadata
    );
    Ok(())
}
