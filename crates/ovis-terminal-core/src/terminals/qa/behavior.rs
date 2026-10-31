//! Named task-16 acceptance cases against real process, store and authority behavior.

use std::io::{self, Write};

use ovis_terminal_core::registry::{ProcessHandle, ProcessIdentity, ProcessStartToken};
use ovis_terminal_core::terminals::KillRequest;
use ovis_terminal_holder::process::{ProcessId, read_start_token};

use super::*;

/// Executes the plan's real launch, removal and kill acceptance case.
///
/// # Errors
///
/// Returns native holder, authority or store failures.
///
/// # Panics
///
/// Panics when an observed lifecycle fact violates the acceptance contract.
pub fn happy() -> TestResult {
    // Given: a real holder and a symlinked cwd with explicit environment, argv and grid.
    let mut fixture = Fixture::new()?;
    let requested_cwd = fixture.root.join("cwd-alias");
    std::os::unix::fs::symlink(&fixture.root, &requested_cwd)?;
    let launch = Fixture::launch_in([
        "/bin/sh", "-c",
        "printf 'cwd=%s\\nenv=%s\\narg=%s\\n' \"$PWD\" \"$OVIS_FIXTURE\" \"$1\"; stty size; printf '\\303\\251'; exit 7",
        "fixture", "literal ; argument",
    ].map(str::to_owned).to_vec(), requested_cwd)?;

    // When: the launch is admitted and actual holder evidence is consumed.
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let admitted = runtime.run(call(&fixture.session), launch)?;
    assert_eq!(admitted.spawn, LaunchState::Pending);
    assert_eq!(admitted.lifecycle.process(), None);
    let ended = fixture.finish(admitted.id())?;
    let bytes = fixture
        .runtime
        .as_ref()
        .ok_or("runtime")?
        .output(&fixture.session, ended.id())?
        .bytes;

    // Then: the program, rather than locally assigned metadata, proves the launch inputs.
    let canonical_cwd = std::fs::canonicalize(&fixture.root)?;
    let expected = format!(
        "cwd={}\r\nenv=explicit-environment\r\narg=literal ; argument\r\n37 103\r\né",
        canonical_cwd.display(),
    );
    assert_eq!(bytes, expected.as_bytes());
    assert_eq!(
        serde_json::to_value(&ended)?
            .pointer("/launch/program/cwd")
            .and_then(serde_json::Value::as_str),
        canonical_cwd.to_str()
    );
    let exit = ended.exit.as_ref().ok_or("exit evidence")?;
    assert_eq!(exit.code, Some(7));
    assert_eq!(exit.final_offset, u64::try_from(bytes.len())?);
    assert!(ended.hangup.is_some());
    writeln!(io::stdout(), "terminal={}", serde_json::to_string(&ended)?)?;
    writeln!(
        io::stdout(),
        "output_bytes={}",
        serde_json::to_string(&bytes)?
    )?;

    let live = fixture.launch(vec![
        "/bin/sh".into(),
        "-c".into(),
        "printf ARMED; read gate".into(),
    ])?;
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let sibling = runtime.run(call(&fixture.session), live)?;
    let sibling = runtime.await_spawn(sibling.id(), WAIT)?;
    let process = sibling.lifecycle.process().ok_or("live process")?.clone();
    runtime.rm(call(&fixture.session), ended.id())?;
    assert_eq!(
        read_start_token(ProcessId::new(u32::try_from(process.pid().get())?))?.as_str(),
        process.start_token().as_str(),
    );
    let killed = runtime.kill(
        call(&fixture.session),
        &KillRequest::immediate(sibling.id(), process.clone()),
    )?;
    assert_eq!(killed.process, process);
    assert!(
        killed
            .exit
            .as_ref()
            .is_some_and(|exit| exit.exit_unknown.is_none())
    );
    writeln!(
        io::stdout(),
        "kill_receipt={}",
        serde_json::to_string(&killed)?
    )?;
    Ok(())
}

/// Executes stale-process rejection and real executable-not-found failure.
///
/// # Errors
///
/// Returns native holder, authority or store failures.
///
/// # Panics
///
/// Panics when stale-process or spawn-failure evidence violates the acceptance contract.
pub fn failure() -> TestResult {
    // Given: a real live process with kernel start-token evidence.
    let mut fixture = Fixture::new()?;
    let launch = fixture.launch(vec![
        "/bin/sh".into(),
        "-c".into(),
        "printf ARMED; read gate".into(),
    ])?;
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let admitted = runtime.run(call(&fixture.session), launch)?;
    let live = runtime.await_spawn(admitted.id(), WAIT)?;
    let current = live.lifecycle.process().ok_or("process")?.clone();
    let stale = ProcessIdentity::new(
        current.process_id(),
        current.generation(),
        ProcessHandle::new(current.pid(), ProcessStartToken::new("stale-start")?),
    );

    // When: a kill names the same PID with a stale start token.
    let error = runtime
        .kill(
            call(&fixture.session),
            &KillRequest::immediate(live.id(), stale),
        )
        .err()
        .ok_or("stale process was not rejected")?;

    // Then: no signal reaches the actual process; it retains the original kernel token.
    assert_eq!(error.kind(), "process_replaced");
    assert_eq!(
        read_start_token(ProcessId::new(u32::try_from(current.pid().get())?))?.as_str(),
        current.start_token().as_str()
    );
    writeln!(
        io::stdout(),
        "stale_kind={}; signals_delivered=0",
        error.kind()
    )?;

    let missing = fixture.launch(vec!["/nonexistent/ovis-task-16-fixture".into()])?;
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let admitted = runtime.run(call(&fixture.session), missing)?;
    assert_eq!(admitted.spawn, LaunchState::Pending);
    let failed = fixture.finish(admitted.id())?;
    assert_eq!(failed.spawn, LaunchState::Failed);
    assert_eq!(failed.lifecycle.process(), None);
    assert!(failed.failure.is_some());
    let outcome = fixture
        .runtime
        .as_ref()
        .ok_or("runtime")?
        .operation(&fixture.session, &failed.launch_op)?;
    assert_eq!(
        outcome.error.ok_or("typed rejection")?.kind.token(),
        "spawn_failed"
    );
    writeln!(
        io::stdout(),
        "spawn_failure={}",
        serde_json::to_string(&failed.failure)?
    )?;
    Ok(())
}
