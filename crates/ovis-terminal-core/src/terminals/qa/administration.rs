//! Explicit program-stop scope and Linux foreground/background observations.

use ovis_terminal_core::terminals::ControlReceipt;
#[cfg(target_os = "linux")]
use ovis_terminal_core::terminals::Field;

use super::*;

#[cfg(target_os = "linux")]
#[test]
fn child_agent_exit_invalidates_authority_without_marking_the_shell_exited() -> TestResult {
    // Given: a child owns the declared agent role while its launching shell remains alive.
    let mut fixture = Fixture::new()?;
    let launch = fixture.launch(vec!["/bin/sh".into(), "-c".into(),
        "mkfifo guard; exec 3<>guard; set -m; { printf BG_READY; exec cat; } <guard 3>&- & child=$!; read gate; exec 3>&-; wait \"$child\"; printf CHILD_EXIT; read hold".into()])?;
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let admitted = runtime.run(call(&fixture.session), launch)?;
    let original = runtime.await_spawn(admitted.id(), WAIT)?;
    fixture.marker(original.id(), b"BG_READY")?;
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let info = runtime.process_info(&fixture.session, original.id())?;
    let Field::Known(background) = info.background else {
        return Err("background evidence".into());
    };
    let owner = background
        .into_iter()
        .find(|sample| {
            original
                .lifecycle
                .process()
                .is_some_and(|root| sample.process.pid() != root.pid())
        })
        .ok_or("child process")?
        .process;
    runtime.bind_agent(call(&fixture.session), (original.id(), owner.clone()))?;
    runtime.check_agent(&fixture.session, (original.id(), &owner))?;

    // When: the parent closes the child lifeline, reaps it, and reports the exact barrier.
    let target = original.holder_target.clone().ok_or("holder target")?;
    fixture
        .holder
        .input(ovis_terminal_holder::protocol::Input {
            grant: ovis_terminal_holder::protocol::WriteGrant {
                grant_epoch: 1,
                fence_epoch: 1,
                process_incarnation: target.process_incarnation.ok_or("process")?,
            },
            target,
            sequence: 1,
            bytes: b"release\n".to_vec(),
        })?;
    fixture.marker(original.id(), b"CHILD_EXIT")?;
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let rejected = runtime
        .check_agent(&fixture.session, (original.id(), &owner))
        .expect_err("child exited");

    // Then: agent-only actions are fenced, but the independently owned shell still runs.
    assert_eq!(rejected.kind(), "process_replaced");
    let current = runtime.get(&fixture.session, original.id())?;
    assert_eq!(current.agent_owner, None);
    assert_eq!(current.lifecycle.process(), original.lifecycle.process());
    assert_eq!(current.exit, None);
    Ok(())
}

#[test]
fn program_stop_names_every_reaped_incarnation_and_leaves_control_available() -> TestResult {
    // Given: two independently owned actual programs.
    let mut fixture = Fixture::new()?;
    let mut processes = Vec::new();
    for _ in 0..2 {
        let launch = fixture.launch(vec![
            "/bin/sh".into(),
            "-c".into(),
            "printf ARMED; read gate".into(),
        ])?;
        let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
        let admitted = runtime.run(call(&fixture.session), launch)?;
        let record = runtime.await_spawn(admitted.id(), WAIT)?;
        processes.push(record.lifecycle.process().ok_or("process")?.clone());
        fixture.marker(record.id(), b"ARMED")?;
    }

    // When: the explicit program-stop action runs, without stopping control.
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let receipt = runtime.program_stop(call(&fixture.session))?;

    // Then: only actual exits are named ended, and a later launch is still accepted.
    let ControlReceipt::ProgramStop {
        ended,
        unknown,
        pending,
    } = receipt
    else {
        return Err("wrong stop scope".into());
    };
    assert_eq!(ended, processes);
    assert_eq!(unknown, Vec::new());
    assert_eq!(pending, Vec::new());
    let next = fixture.launch(vec!["/bin/sh".into(), "-c".into(), "exit 0".into()])?;
    let record = fixture
        .runtime
        .as_mut()
        .ok_or("runtime")?
        .run(call(&fixture.session), next)?;
    assert_eq!(
        fixture.finish(record.id())?.exit.ok_or("exit")?.code,
        Some(0)
    );
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn native_process_roles_have_start_tokens_without_shell_or_agent_guessing() -> TestResult {
    // Given: a real shell with job control and a background process on its PTY.
    let mut fixture = Fixture::new()?;
    let launch = fixture.launch(vec![
        "/bin/sh".into(), "-c".into(),
        "mkfifo guard; exec 3<>guard; set -m; { printf BG_READY; exec cat; } <guard 3>&- & printf ARMED; read gate; exec 3>&-; wait".into(),
    ])?;
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let admitted = runtime.run(call(&fixture.session), launch)?;
    let record = runtime.await_spawn(admitted.id(), WAIT)?;
    fixture.marker(record.id(), b"BG_READY")?;

    // When: native roles are sampled for this exact terminal.
    let info = fixture
        .runtime
        .as_mut()
        .ok_or("runtime")?
        .process_info(&fixture.session, record.id())?;

    // Then: the foreground and background differ, and every process has PID plus start token.
    let Field::Known(foreground) = info.foreground else {
        return Err("foreground unsupported".into());
    };
    let Field::Known(background) = info.background else {
        return Err("background unsupported".into());
    };
    let expected = record.lifecycle.process().ok_or("launcher")?;
    assert!(
        foreground
            .iter()
            .any(|sample| sample.process.pid() == expected.pid())
    );
    assert!(
        background
            .iter()
            .any(|sample| sample.process.pid() != expected.pid())
    );
    assert!(
        foreground
            .iter()
            .chain(background.iter())
            .all(|sample| !sample.process.start_token().as_str().is_empty())
    );
    assert!(matches!(
        info.shell,
        Field::Unsupported(ovis_terminal_core::terminals::Unavailable::UndeclaredRole)
    ));
    Ok(())
}
