//! Real holder delivery and exit evidence cases.

use std::io::Write as _;
use std::time::Duration;

use ovis_terminal_holder::protocol::{
    CoreBuildId, HOLDER_PROTOCOL_GENERATION, Hello, HolderBuildId, HolderDescriptor,
    HolderInstanceId, Input, Message, PreviousCoreState, Spawn, Spawned, TerminalId,
    TerminalIncarnation, TerminalTarget, WriteGrant,
};
use ovis_terminal_holder::unix::{CoreLink, Holder, HolderConfig, HolderLimits};
use ovis_terminal_protocol::schema::methods::server::OpRecord;

use super::{Fixture, Operations, TestResult, context, request};

struct ManagedHolder(Holder);

impl Drop for ManagedHolder {
    fn drop(&mut self) {
        let shutdown = self.0.shutdown();
        assert!(shutdown.is_ok(), "holder workers must join: {shutdown:?}");
    }
}

/// One actual PTY write with the acknowledgement and process exit joined to its receipt.
///
/// # Errors
///
/// Returns real holder, store or evidence failures instead of a simulated receipt.
pub fn perform(
    operations: &Operations<'_, super::OperationEvent, super::OperationProducer>,
) -> Result<OpRecord, Box<dyn std::error::Error>> {
    let permit = deliver(operations)?;
    Ok(operations.commit(permit, serde_json::json!({"delivered_bytes":"4"}))?)
}

impl ManagedHolder {
    fn start() -> Result<(Self, CoreLink, Spawned), Box<dyn std::error::Error>> {
        // Given a holder that reports a readiness marker before reading one input line.
        let managed = Self(Holder::new(HolderConfig::new(
            HolderDescriptor::new(
                HolderBuildId::new("ops-test".into()),
                HolderInstanceId::new(uuid::Uuid::now_v7().to_string()),
            ),
            HolderLimits::DEFAULT,
        )));
        let holder = &managed.0;
        let link = holder.adopt(&Hello {
            generation: HOLDER_PROTOCOL_GENERATION,
            core_build: CoreBuildId::new("ops-core".into()),
            previous: PreviousCoreState::Crash,
        })?;
        let spawned = holder.spawn_wire(&Spawn {
        terminal_id: TerminalId::new("terminal:1".into()),
        terminal_incarnation: TerminalIncarnation::new(1),
        argv: vec![
            "/bin/sh".into(),
            "-c".into(),
            "stty -echo; printf 'ARMED\\n'; IFS= read -r line; printf 'EFFECT:%s\\n' \"$line\"; IFS= read -r gate"
                .into(),
        ],
        cwd: std::env::temp_dir(),
        env: [
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("TERM".into(), "xterm".into()),
        ]
        .into(),
        geometry: (80, 24),
    })?;
        Ok((managed, link, spawned))
    }
}

fn deliver(
    operations: &Operations<'_, super::OperationEvent, super::OperationProducer>,
) -> Result<ovis_terminal_core::ops::RunningOperation, Box<dyn std::error::Error>> {
    let (managed, link, spawned) = ManagedHolder::start()?;
    let holder = &managed.0;
    let wait = Duration::from_secs(30);
    let mut bytes = Vec::new();
    while !bytes.windows(5).any(|window| window == b"ARMED") {
        if let Message::Output(output) = link.recv_timeout(wait)? {
            bytes.extend(output.bytes);
        }
    }
    let req = request("one")?;
    let permit = operations.start(req.op_id())?;
    let input = Input {
        target: TerminalTarget {
            terminal_id: spawned.terminal_id,
            terminal_incarnation: spawned.terminal_incarnation,
            process_incarnation: Some(spawned.process_incarnation),
        },
        sequence: 1,
        grant: WriteGrant {
            grant_epoch: 1,
            fence_epoch: 1,
            process_incarnation: spawned.process_incarnation,
        },
        bytes: b"one\n".to_vec(),
    };
    operations.bind_input(&permit, &input)?;

    // When the real holder writes the input and then reports process exit.
    let release_input = Input {
        sequence: 2,
        bytes: b"quit\n".to_vec(),
        ..input.clone()
    };
    holder.input(input)?;
    let mut wrote = false;
    let mut exited = false;
    while !wrote || !exited {
        match link.recv_timeout(wait)? {
            Message::Output(output) => bytes.extend(output.bytes),
            Message::InputWritten(ack) if ack.sequence == 1 => {
                let mut foreign = ack.clone();
                foreign.sequence = 2;
                let Err(error) = operations.holder_write(req.op_id(), &foreign) else {
                    return Err("foreign input entry was accepted".into());
                };
                assert_eq!(error.kind(), "stale_target");
                foreign = ack.clone();
                foreign.target.terminal_id = TerminalId::new("terminal:other".into());
                let Err(error) = operations.holder_write(req.op_id(), &foreign) else {
                    return Err("foreign input target was accepted".into());
                };
                assert_eq!(error.kind(), "stale_target");
                let recorded = operations.holder_write(req.op_id(), &ack)?;
                assert!(
                    recorded
                        .evidence
                        .as_ref()
                        .is_some_and(|evidence| evidence.exit.is_none())
                );
                assert_eq!(ack.written_bytes, 4);
                wrote = true;
                holder.input(release_input.clone())?;
            }
            Message::Exit(exit) => {
                let recorded = operations.holder_exit(req.op_id(), &exit)?;
                assert!(
                    recorded
                        .evidence
                        .as_ref()
                        .and_then(|evidence| evidence.exit.as_ref())
                        .is_some_and(|exit| exit.observed && exit.code == Some(0))
                );
                exited = true;
            }
            _ => {}
        }
    }

    // Then the receipt proves delivery and actual exit, with exactly one effect marker.
    assert_eq!(
        bytes
            .windows(10)
            .filter(|window| *window == b"EFFECT:one")
            .count(),
        1
    );
    writeln!(
        std::io::stdout().lock(),
        "holder_bytes={}",
        serde_json::to_string(&bytes)?
    )?;
    Ok(permit)
}

#[test]
fn holder_receipt_and_evidence_survive_identical_retry_and_reopen() -> TestResult {
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let operations = Operations::new(&core, context(1)?);
    let req = request("one")?;
    operations.accept(&req)?;
    let receipt = perform(&operations)?;
    assert_eq!(operations.accept(&req)?, receipt);
    core.close()?;
    let core = fixture.open(2)?;
    let operations = Operations::new(&core, context(2)?);
    assert_eq!(operations.accept(&req)?, receipt);
    core.close()?;
    Ok(())
}

#[test]
fn unknown_write_never_becomes_a_committed_success() -> TestResult {
    // Given persisted intent with no holder acknowledgement.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let operations = Operations::new(&core, context(1)?);
    let req = request("one")?;
    operations.accept(&req)?;
    let permit = operations.start(req.op_id())?;
    let input: Input = serde_json::from_value(serde_json::json!({
        "target":{"terminal_id":"terminal:1","terminal_incarnation":"1","process_incarnation":"1"},
        "sequence":"1","grant":{"grant_epoch":"1","fence_epoch":"1","process_incarnation":"1"},
        "bytes":[111,110,101,10]
    }))?;
    operations.bind_input(&permit, &input)?;

    // When a caller attempts to claim completion without any delivery proof.
    let rejected = operations
        .commit(permit, serde_json::json!({"delivered":true}))
        .expect_err("no proof");

    // Then the typed rejection and durable unknown state prohibit automatic replay.
    assert_eq!(rejected.kind(), "outcome_unknown");
    assert_eq!(
        operations.get(req.op_id())?.state,
        super::OpenEnum::Known(super::OpState::OutcomeUnknown)
    );
    core.close()?;
    Ok(())
}

#[test]
fn acknowledged_write_survives_crash_before_method_completion() -> TestResult {
    // Given real holder delivery whose evidence committed before the caller disappeared.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let operations = Operations::new(&core, context(1)?);
    let req = request("one")?;
    operations.accept(&req)?;
    let _interrupted = deliver(&operations)?;
    core.close()?;

    // When the new core inspects the uncompleted operation.
    let core = fixture.open(2)?;
    let operations = Operations::new(&core, context(2)?);
    let unknown = operations.get(req.op_id())?;

    // Then uncertainty never discards already proven write and exit facts or grants replay.
    assert_eq!(
        unknown.state,
        super::OpenEnum::Known(super::OpState::OutcomeUnknown)
    );
    assert!(
        unknown
            .evidence
            .as_ref()
            .and_then(|evidence| evidence.write.as_ref())
            .is_some_and(|write| write.written_bytes.get() == 4)
    );
    assert_eq!(
        operations.start(req.op_id()).expect_err("no resend").kind(),
        "outcome_unknown"
    );
    assert_eq!(operations.accept(&req)?, unknown);
    core.close()?;
    Ok(())
}
