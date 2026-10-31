//! Revocation occurs at an actual store barrier, before the holder effect.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, mpsc};

use ovis_terminal_authz::Revocation;
use ovis_terminal_holder::protocol::{
    CoreBuildId, HOLDER_PROTOCOL_GENERATION, Hello, Message, PreviousCoreState,
};
use ovis_terminal_store::CommitBarrier;

use super::*;

#[test]
fn revocation_after_admission_prevents_spawn_and_commits_typed_failure() -> TestResult {
    // Given: the real writer pauses after the terminal-admission unit committed.
    let (reached, wait) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::sync_channel(1);
    let gate = Mutex::new(gate);
    let commits = AtomicUsize::new(0);
    let options = StoreOptions::default().with_barrier(move |barrier| {
        if barrier == CommitBarrier::Committed && commits.fetch_add(1, Ordering::SeqCst) == 2 {
            reached.send(()).expect("admission reached");
            gate.lock()
                .expect("release gate")
                .recv_timeout(WAIT)
                .expect("bounded release");
        }
    });
    let mut fixture = Fixture::with_options(options)?;
    let launch = fixture.launch(vec![
        "/bin/sh".into(),
        "-c".into(),
        "printf UNAUTHORIZED".into(),
    ])?;
    let runtime = fixture.runtime.as_mut().ok_or("runtime")?;
    let session = &fixture.session;

    // When: the credential is revoked while the admitted launch is paused.
    let admitted = std::thread::scope(|scope| -> Result<Record, Box<dyn std::error::Error>> {
        let worker = scope.spawn(|| runtime.run(call(session), launch));
        let arrival = wait.recv_timeout(WAIT);
        let revoked = fixture.authority.revoke(
            session,
            &Revocation::new(fixture.credential, TimestampMs::new(2)),
        );
        release.send(())?;
        arrival?;
        revoked?;
        Ok(worker.join().map_err(|_| "launch panicked")??)
    })?;
    let rejected = runtime
        .receive(WAIT)
        .expect_err("write-boundary authority rejected");

    // Then: no program exists, and the same atomic unit retains the failed terminal and operation.
    assert_eq!(rejected.kind(), "credential_revoked");
    let snapshot = runtime.coordinator().read_snapshot()?;
    let record = snapshot
        .sections
        .terminals
        .iter()
        .find(|record| record.id() == admitted.id())
        .ok_or("record")?;
    assert_eq!(record.spawn, LaunchState::Failed);
    assert_eq!(record.lifecycle.process(), None);
    assert_eq!(
        record
            .rejection
            .as_ref()
            .ok_or("typed rejection")?
            .kind
            .token(),
        "credential_revoked"
    );
    let at = TimestampMs::new(u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?);
    let operations = ovis_terminal_core::ops::Operations::new(
        runtime.coordinator(),
        ovis_terminal_core::ops::Context::new(session.principal().clone(), fixture.identity, at),
    );
    assert_eq!(
        operations
            .get(&record.launch_op)?
            .error
            .ok_or("operation failure")?
            .kind
            .token(),
        "credential_revoked"
    );
    let link = fixture.holder.adopt(&Hello {
        generation: HOLDER_PROTOCOL_GENERATION,
        core_build: CoreBuildId::new("inspect".into()),
        previous: PreviousCoreState::Crash,
    })?;
    let Message::Welcome(welcome) = link.recv_timeout(WAIT)? else {
        return Err("welcome".into());
    };
    assert_eq!(welcome.terminals, Vec::new());
    link.release();
    Ok(())
}
