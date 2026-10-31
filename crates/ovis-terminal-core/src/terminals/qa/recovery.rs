//! Interrupted launch observation and independently proven holder loss.

use ovis_terminal_core::registry::TerminalState;
use ovis_terminal_protocol::schema::methods::server::OpState;
use ovis_terminal_protocol::schema::scalar::OpenEnum;

use super::*;

#[test]
fn pending_launch_is_adopted_without_repeating_the_external_effect() -> TestResult {
    // Given: admission committed, but this control has not consumed Spawned.
    let mut fixture = Fixture::new()?;
    let launch = fixture.launch(vec![
        "/bin/sh".into(),
        "-c".into(),
        "printf ARMED; read gate".into(),
    ])?;
    let operation = OpId::new(Uuid::now_v7().to_string());
    let admitted = fixture.runtime.as_mut().ok_or("runtime")?.run(
        Call::new(&fixture.session, operation.clone()),
        launch.clone(),
    )?;
    assert_eq!(admitted.spawn, LaunchState::Pending);

    // When: the control lifetime ends and a successor adopts the actual holder.
    fixture.reopen(2)?;
    fixture.marker(admitted.id(), b"ARMED")?;
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let adopted = runtime.get(&fixture.session, admitted.id())?;
    let replay = runtime.run(Call::new(&fixture.session, operation.clone()), launch)?;

    // Then: the same terminal has one proven process, while the interrupted op remains honest.
    assert_eq!(adopted.spawn, LaunchState::Spawned);
    assert_eq!(adopted.lifecycle.process(), replay.lifecycle.process());
    assert_eq!(runtime.list(&fixture.session)?.len(), 1);
    assert_eq!(
        runtime.operation(&fixture.session, &operation)?.state,
        OpenEnum::Known(OpState::OutcomeUnknown)
    );
    assert_eq!(
        runtime.output(&fixture.session, adopted.id())?.bytes,
        b"ARMED"
    );
    Ok(())
}

#[test]
fn holder_loss_recovery_requires_absence_of_the_old_process() -> TestResult {
    // Given: a kept process whose holder dies while its old control retains live metadata.
    let mut fixture = Fixture::new()?;
    let mut launch = fixture.launch(vec![
        "/bin/sh".into(),
        "-c".into(),
        "printf ARMED; read gate".into(),
    ])?;
    launch.keep = true;
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let admitted = runtime.run(call(&fixture.session), launch)?;
    let original = runtime.await_spawn(admitted.id(), WAIT)?;
    fixture.marker(original.id(), b"ARMED")?;
    fixture.runtime.take().ok_or("runtime")?.close()?;
    fixture.holder.shutdown()?;
    fixture.holder = Holder::new(HolderConfig::new(
        HolderDescriptor::new(
            HolderBuildId::new("replacement".into()),
            HolderInstanceId::new(Uuid::now_v7().to_string()),
        ),
        HolderLimits::DEFAULT,
    ));
    fixture.identity = RuntimeIdentity::new(
        HostIdentity::new(
            fixture.identity.machine_id(),
            fixture.identity.host_boot_id(),
        ),
        fixture.identity.runtime_id(),
        ServerBootId::try_from(2)?,
    );

    // When: a new holder is adopted and explicit recovery is requested.
    fixture.runtime = Some(Terminals::new(
        Store::open(&fixture.root.join("state"), StoreOptions::default())?,
        Host::new(
            fixture.identity,
            Arc::clone(&fixture.authority),
            fixture.holder.clone(),
        ),
    )?);
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let lost = runtime.get(&fixture.session, original.id())?;
    assert_eq!(lost.lifecycle.state(), TerminalState::Lost);
    assert_eq!(lost.lifecycle.process(), None);
    runtime.recover(call(&fixture.session), original.id())?;
    let recovered = runtime.await_spawn(original.id(), WAIT)?;

    // Then: the terminal identity is retained, with a new incarnation and historical old proof.
    assert_eq!(recovered.id(), original.id());
    assert_eq!(
        recovered.lifecycle.generation(),
        original.lifecycle.generation().checked_next()?
    );
    assert_eq!(
        recovered.lifecycle.previous_process(),
        original.lifecycle.process()
    );
    assert_ne!(recovered.lifecycle.process(), original.lifecycle.process());
    assert_eq!(
        recovered.lifecycle.identity().server_boot_id(),
        fixture.identity.server_boot_id()
    );
    Ok(())
}
