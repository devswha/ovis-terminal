//! Admission: identity, readiness, encoding and bounds, before any write.

use ovis_terminal_authz::{ResolvedTarget, TargetResource};
use ovis_terminal_core::input::encoding::InputKind;
use ovis_terminal_core::input::receipts::{Delivery, Stage};
use ovis_terminal_core::input::sequencer::{Expected, Payload, QueueLimits, Sequencer};
use ovis_terminal_core::registry::{MachineId, RuntimeId, TerminalGeneration, TerminalId};
use ovis_terminal_protocol::schema::methods::input::Submit;
use uuid::Uuid;

use super::TestResult;
use super::fixture::{Fixture, WRITER, admit, modes, process, submission, view};

#[test]
fn text_and_submission_are_one_queued_entry_answered_before_any_write() -> TestResult {
    // Given a live terminal and a writer with the input grant.
    let fixture = Fixture::new()?;
    let (leases, modes) = (fixture.leases()?, modes(b"")?);
    let view = view(&leases, &modes);
    let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;
    let alice = fixture.client("alice", WRITER)?;

    // When text with Enter is admitted.
    let payload = Payload::Text {
        text: "hello",
        submit: &Submit::Enter,
    };
    let receipt = admit(
        &mut sequencer,
        &view,
        (
            alice.writer(),
            submission("op-1", fixture.expected()?, payload),
        ),
    )?;

    // Then the receipt is an admission only, and the write carries text and Enter together.
    assert_eq!(receipt.delivery, Delivery::Queued);
    assert_eq!(receipt.queued_seq.get(), 1);
    assert_eq!(receipt.kind, InputKind::Text);
    assert_eq!(receipt.admitted_bytes.get(), 6);
    assert_eq!(
        [
            receipt.agent_accepted,
            receipt.turn_started,
            receipt.turn_ended
        ],
        [Stage::Unknown; 3]
    );
    let input = sequencer.next(&view)?.dispatch.ok_or("no write")?;
    assert_eq!(input.bytes, b"hello\r");
    assert!(
        sequencer.next(&view)?.dispatch.is_none(),
        "one user entry with the holder at a time"
    );
    Ok(())
}

#[test]
fn moved_identities_are_typed_and_queue_nothing() -> TestResult {
    // Given a live terminal running process 1.
    let fixture = Fixture::new()?;
    let (leases, modes) = (fixture.leases()?, modes(b"")?);
    let view = view(&leases, &modes);
    let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;
    let alice = fixture.client("alice", WRITER)?;
    let payload = Payload::Text {
        text: "x",
        submit: &Submit::None,
    };
    let other = |terminal: u64, incarnation: u64| -> TestResult<Expected> {
        let resource = TargetResource::Terminal {
            placement: None,
            terminal: TerminalId::try_from(terminal)?,
            incarnation: TerminalGeneration::new(incarnation)?,
        };
        Ok(Expected::new(
            RuntimeId::try_from(1)?,
            ResolvedTarget::new(*fixture.target().machine(), resource),
        ))
    };
    let foreign_machine = Expected::new(
        RuntimeId::try_from(1)?,
        ResolvedTarget::new(
            MachineId::new(Uuid::from_u128(99))?,
            fixture.target().resource().clone(),
        ),
    );

    // When requests resolved against other identities are admitted.
    let cases = [
        (
            Expected::new(RuntimeId::try_from(2)?, fixture.target()),
            "stale_target",
        ),
        (foreign_machine, "stale_target"),
        (other(10, 1)?, "stale_target"),
        (other(9, 2)?, "stale_target"),
        (
            fixture.expected()?.with_process(process(2)?),
            "process_replaced",
        ),
    ];
    for (expected, kind) in cases {
        let refused = admit(
            &mut sequencer,
            &view,
            (alice.writer(), submission("op", expected, payload)),
        );
        // Then each is refused with its kind.
        assert_eq!(refused.map(|_| ()).map_err(|error| error.kind()), Err(kind));
    }

    // And nothing reaches the holder.
    assert!(sequencer.next(&view)?.dispatch.is_none());
    Ok(())
}

#[test]
fn readiness_gates_admission_but_agent_readiness_does_not() -> TestResult {
    // Given a terminal whose stream is not ready yet.
    let fixture = Fixture::new()?;
    let (leases, modes) = (fixture.leases()?, modes(b"")?);
    let view = view(&leases, &modes);
    let alice = fixture.client("alice", WRITER)?;
    let payload = Payload::Text {
        text: "x",
        submit: &Submit::None,
    };
    let send = |sequencer: &mut Sequencer| -> TestResult<Result<(), &'static str>> {
        let request = (
            alice.writer(),
            submission("op", fixture.expected()?, payload),
        );
        Ok(admit(sequencer, &view, request)
            .map(|_| ())
            .map_err(|error| error.kind()))
    };
    let mut sequencer = Sequencer::new(fixture.binding()?, QueueLimits::DEFAULT);

    // When input arrives before, during and after the live stream.
    assert_eq!(send(&mut sequencer)?, Err("not_ready"));
    sequencer.stream_ready(
        process(1)?,
        ovis_terminal_protocol::schema::methods::server::WriteSequence::new(0),
    );
    // Then an ordinary send needs no agent-declared readiness once the stream is live.
    assert_eq!(send(&mut sequencer)?, Ok(()));
    sequencer.link_lost();
    assert_eq!(send(&mut sequencer)?, Err("recovering"));
    sequencer.exited();
    assert_eq!(send(&mut sequencer)?, Err("finished"));
    Ok(())
}

#[test]
fn encoding_and_bound_refusals_come_before_the_queue() -> TestResult {
    // Given a terminal without bracketed paste and a queue of 8 bytes in 2 entries.
    let fixture = Fixture::new()?;
    let (leases, modes) = (fixture.leases()?, modes(b"")?);
    let view = view(&leases, &modes);
    let mut sequencer = fixture.sequencer(QueueLimits::new(8, 2))?;
    let alice = fixture.client("alice", WRITER)?;
    let keys = [String::from("hyper+x")];
    let none = Submit::None;
    let mut send = |op: &str, payload| -> TestResult<Result<u64, &'static str>> {
        let request = (alice.writer(), submission(op, fixture.expected()?, payload));
        Ok(admit(&mut sequencer, &view, request)
            .map(|receipt| receipt.queued_seq.get())
            .map_err(|error| error.kind()))
    };

    // When refused and accepted requests alternate.
    assert_eq!(
        send(
            "a",
            Payload::Text {
                text: "a\nb",
                submit: &none
            }
        )?,
        Err("paste_refused")
    );
    assert_eq!(send("b", Payload::Keys(&keys))?, Err("unknown_key"));
    assert_eq!(
        send(
            "c",
            Payload::Text {
                text: "12345",
                submit: &none
            }
        )?,
        Ok(1)
    );
    assert_eq!(
        send(
            "d",
            Payload::Text {
                text: "1234",
                submit: &none
            }
        )?,
        Err("input_overflow")
    );
    assert_eq!(
        send(
            "e",
            Payload::Text {
                text: "123",
                submit: &none
            }
        )?,
        Ok(2)
    );
    // Then refusals consumed no position and the entry bound holds too.
    assert_eq!(
        send(
            "f",
            Payload::Text {
                text: "",
                submit: &Submit::Enter
            }
        )?,
        Err("input_overflow")
    );
    Ok(())
}
