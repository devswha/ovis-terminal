//! Competing execution claims and repeated core interruptions.

use std::io::Write as _;
use std::sync::{Arc, Barrier};

use super::{Fixture, OpState, OpenEnum, Operations, TestResult, context, request};

#[test]
fn competing_starts_grant_exactly_one_external_effect() -> TestResult {
    // Given a durable acceptance and eight registered contenders.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let req = request("concurrent")?;
    Operations::new(&core, context(1)?).accept(&req)?;
    let barrier = Arc::new(Barrier::new(8));
    let effect = fixture.0.join("concurrent-effect");

    // When all contenders race the running transition.
    std::thread::scope(|scope| -> TestResult {
        let mut contenders = Vec::new();
        for _ in 0..8 {
            let barrier = Arc::clone(&barrier);
            let core = &core;
            let req = &req;
            let effect = &effect;
            contenders.push(scope.spawn(move || {
                let operations = Operations::new(core, context(1).expect("context"));
                barrier.wait();
                match operations.start(req.op_id()) {
                    Ok(permit) => {
                        let mut output = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(effect)
                            .expect("effect file");
                        output.write_all(b"effect\n").expect("effect write");
                        operations
                            .commit(permit, serde_json::json!({"effect":true}))
                            .expect("completion");
                    }
                    Err(error) => assert!(["conflict", "already_started"].contains(&error.kind())),
                }
            }));
        }
        for contender in contenders {
            contender.join().expect("contender joined");
        }
        Ok(())
    })?;

    // Then the real file records one effect and the final receipt is committed.
    assert_eq!(std::fs::read(effect)?, b"effect\n");
    assert_eq!(
        Operations::new(&core, context(1)?).get(req.op_id())?.state,
        OpenEnum::Known(OpState::Committed)
    );
    core.close()?;
    Ok(())
}

#[test]
fn repeated_core_interruptions_never_grant_uncertain_work_again() -> TestResult {
    // Given an effect whose completion receipt was lost.
    let fixture = Fixture::new();
    let req = request("interrupted")?;
    let core = fixture.open(1)?;
    let operations = Operations::new(&core, context(1)?);
    operations.accept(&req)?;
    let _interrupted = operations.start(req.op_id())?;
    std::fs::write(fixture.0.join("effect"), b"only once")?;
    core.close()?;

    // When multiple later core boots inspect and retry the same ID.
    for boot in 2..6 {
        let core = fixture.open(boot)?;
        let operations = Operations::new(&core, context(boot)?);
        let unknown = operations.get(req.op_id())?;

        // Then every boot retains uncertainty and refuses to grant another effect.
        assert_eq!(unknown.state, OpenEnum::Known(OpState::OutcomeUnknown));
        assert_eq!(operations.accept(&req)?, unknown);
        assert_eq!(
            operations
                .start(req.op_id())
                .expect_err("never replay")
                .kind(),
            "outcome_unknown"
        );
        core.close()?;
    }
    assert_eq!(std::fs::read(fixture.0.join("effect"))?, b"only once");
    Ok(())
}
