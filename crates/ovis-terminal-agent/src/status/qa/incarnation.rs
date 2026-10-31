//! Output and exits of a replaced program never touch the current program's status.

use ovis_terminal_protocol::domain::agent_status::IngestCounters;
use ovis_terminal_protocol::schema::records::{AgentState, StatusReport};
use ovis_terminal_protocol::schema::scalar::{Incarnation, OpenEnum, ReportSeq};
use ovis_terminal_vt::{Geometry, HistoryLimit, Vt, VtConfig};

use super::*;
use crate::status::{Ingested, OscOutput, StatusError};

const NEXT_TOKEN: &str = "token-agent-4-incarnation-2";

fn owner(process: u64) -> Result<ProcessOwner, Box<dyn Error>> {
    Ok(ProcessOwner::new(
        terminal(AGENT)?,
        Incarnation::new(process),
    ))
}

fn ingest_done(h: &Harness, process: u64) -> Result<Ingested, Box<dyn Error>> {
    let mut vt = Vt::new(VtConfig::new(
        Geometry::new(80, 24)?,
        HistoryLimit::new(100),
    ));
    let records = vt.feed(b"\x1b]7501;state=done\x1b\\").osc;
    let record = records.first().ok_or("one OSC record")?;
    Ok(h.service
        .ingest(&h.core, OscOutput::new(owner(process)?, record, now()?))?)
}

/// Incarnation 2 replaces incarnation 1 and reports `working` with its own token.
fn replaced(h: &Harness) -> Result<StatusResult, Box<dyn Error>> {
    h.state(AgentState::Working, 1)?;
    h.service.tokens().bind(&token(NEXT_TOKEN), owner(2)?)?;
    h.report(NEXT_TOKEN, (StatusReport::new(AgentState::Working), 1))?;
    h.status(AGENT)
}

#[test]
fn osc_output_of_a_replaced_program_is_counted_stale_and_changes_nothing() -> TestResult {
    let h = harness()?;
    let before = replaced(&h)?;
    assert_eq!(before.status.process_incarnation, Some(Incarnation::new(2)));

    assert_eq!(ingest_done(&h, 1)?, Ingested::Dropped);

    let after = h.status(AGENT)?;
    assert_eq!(after.status, before.status);
    assert_eq!(after.ingest, IngestCounters::new(0, 0, 1));
    assert_eq!(ingest_done(&h, 2)?, Ingested::Applied);
    Ok(())
}

#[test]
fn osc_output_of_a_replaced_program_cannot_fabricate_a_row_before_its_successor_reports()
-> TestResult {
    // Given: incarnation 1 reported, then incarnation 2 started without reporting yet.
    let h = harness()?;
    h.state(AgentState::Working, 1)?;
    h.service.tokens().bind(&token(NEXT_TOKEN), owner(2)?)?;

    // When: incarnation 1's output still carries a status record.
    let outcome = ingest_done(&h, 1)?;

    // Then: it is refused and incarnation 2 stays unreported.
    assert_eq!(outcome, Ingested::Dropped);
    let status = h.status(AGENT)?.status;
    assert!(!status.reported);
    assert_eq!(status.process_incarnation, Some(Incarnation::new(2)));
    Ok(())
}

#[test]
fn the_exit_of_a_replaced_program_is_refused_and_changes_nothing() -> TestResult {
    let h = harness()?;
    let before = replaced(&h)?;
    let refused = h.service.exited(&h.core, owner(1)?);
    assert!(matches!(refused, Err(StatusError::StaleIncarnation)));
    assert_eq!(h.status(AGENT)?, before);
    h.service.exited(&h.core, owner(2)?)?;
    assert_eq!(h.status(AGENT)?.status.root, None);
    Ok(())
}

#[test]
fn the_exit_of_a_replaced_program_ends_no_turn_before_its_successor_reports() -> TestResult {
    let h = harness()?;
    h.state(AgentState::Working, 1)?;
    h.service.tokens().bind(&token(NEXT_TOKEN), owner(2)?)?;
    let before = h.status(AGENT)?;
    let refused = h.service.exited(&h.core, owner(1)?);
    assert!(matches!(refused, Err(StatusError::StaleIncarnation)));
    let after = h.status(AGENT)?;
    assert_eq!(after, before);
    assert_eq!(after.status.last_turn, None);
    Ok(())
}

#[test]
fn a_stored_row_of_a_newer_program_is_never_read_as_an_older_one() -> TestResult {
    let h = harness()?;
    replaced(&h)?;
    let location = h.service.location(terminal(AGENT)?);
    let mut read = None;
    h.run(|snapshot| {
        read = Some(
            location
                .read(snapshot, Some(Incarnation::new(1)))
                .map(|stored| stored.row),
        );
        h.service.status(snapshot, location.terminal)
    })?;
    assert!(matches!(read, Some(Err(StatusError::StaleIncarnation))));
    Ok(())
}

#[test]
fn an_osc_record_after_the_last_producer_sequence_is_refused() -> TestResult {
    let h = harness()?;
    h.state(AgentState::Working, u64::MAX)?;
    let before = h.status(AGENT)?;
    assert_eq!(ingest_done(&h, 1)?, Ingested::Dropped);
    let after = h.status(AGENT)?;
    assert_eq!(after.status, before.status);
    let root = after.status.root.ok_or("root")?;
    assert_eq!(root.report_seq, ReportSeq::new(u64::MAX));
    assert_eq!(root.report.state, OpenEnum::Known(AgentState::Working));
    assert_eq!(after.ingest, IngestCounters::new(0, 0, 1));
    Ok(())
}
