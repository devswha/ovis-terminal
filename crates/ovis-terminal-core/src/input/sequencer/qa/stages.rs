//! Agent stages: filled only by a declared producer, never by PTY delivery.

use ovis_terminal_core::input::receipts::{AgentStage, Reporter, Stage, StageReport};
use ovis_terminal_core::input::sequencer::{Payload, QueueLimits};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::input::Submit;

use super::TestResult;
use super::fixture::{Fixture, NOW, WRITER, ack, admit, modes, submission, view};

#[test]
fn agent_stages_come_only_from_a_declared_producer_after_delivery() -> TestResult {
    // Given one entry with a declared producer and one without.
    let fixture = Fixture::new()?;
    let (leases, modes) = (fixture.leases()?, modes(b"")?);
    let view = view(&leases, &modes);
    let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;
    let agent = fixture.client("agent", WRITER)?;
    let payload = Payload::Text {
        text: "task",
        submit: &Submit::Enter,
    };
    let declared =
        submission("declared", fixture.expected()?, payload).reported_by(Reporter::Declared);
    let receipt = admit(&mut sequencer, &view, (agent.writer(), declared))?;
    admit(
        &mut sequencer,
        &view,
        (
            agent.writer(),
            submission("plain", fixture.expected()?, payload),
        ),
    )?;
    assert_eq!(receipt.agent_accepted, Stage::Pending);
    let report = |op: &str, stage| StageReport::new(OpId::new(op.to_owned()), stage, NOW);

    // When acceptance is reported before and after the PTY write.
    let early = sequencer.report(&report("declared", AgentStage::Accepted));
    let input = sequencer.next(&view)?.dispatch.ok_or("no write")?;
    sequencer.acknowledge(&ack(&input, 5))?;
    let accepted = sequencer.report(&report("declared", AgentStage::Accepted))?;

    // Then delivery alone filled nothing and reports land only where they can.
    assert_eq!(
        early.map_err(|error| error.kind()).err(),
        Some("invalid_record")
    );
    assert_eq!(accepted.agent_accepted, Stage::Reported { at_ms: NOW });
    assert_eq!(accepted.turn_started, Stage::Pending);
    let unsupported = sequencer.report(&report("plain", AgentStage::TurnStarted));
    assert_eq!(
        unsupported.map_err(|error| error.kind()).err(),
        Some("invalid_record")
    );
    let missing = sequencer.report(&report("missing", AgentStage::TurnEnded));
    assert_eq!(
        missing.map_err(|error| error.kind()).err(),
        Some("not_found")
    );
    Ok(())
}
