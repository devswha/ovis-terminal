//! OSC 7501 ingestion through the real VT engine; screen text has no status path.

use ovis_terminal_protocol::domain::agent_status::{
    DecodeFailure, IngestCounters, OscDecodeError, OscTextField,
};
use ovis_terminal_protocol::schema::records::{AgentState, StatusOrigin};
use ovis_terminal_protocol::schema::scalar::{Incarnation, OpenEnum, ReportSeq};
use ovis_terminal_vt::{Geometry, HistoryLimit, OscRecord, Vt, VtConfig};
use proptest::prelude::*;

use super::*;
use crate::status::{Ingested, OscOutcome, OscOutput, classify};

fn records(chunks: &[&[u8]]) -> Result<Vec<OscRecord>, Box<dyn Error>> {
    let mut vt = Vt::new(VtConfig::new(
        Geometry::new(80, 24)?,
        HistoryLimit::new(100),
    ));
    Ok(chunks.iter().flat_map(|chunk| vt.feed(chunk).osc).collect())
}

fn ingest(h: &Harness, bytes: &[u8]) -> Result<Vec<Ingested>, Box<dyn Error>> {
    let owner = ProcessOwner::new(terminal(AGENT)?, Incarnation::new(1));
    let at = now()?;
    let mut outcomes = Vec::new();
    for record in records(&[bytes])? {
        outcomes.push(
            h.service
                .ingest(&h.core, OscOutput::new(owner, &record, at))?,
        );
    }
    Ok(outcomes)
}

fn osc(payload: &str) -> Vec<u8> {
    format!("\u{1b}]7501;{payload}\u{1b}\\").into_bytes()
}

#[test]
fn fake_status_text_on_screen_never_changes_a_reported_status() -> TestResult {
    // Given: an explicit working report at sequence 4.
    let h = harness()?;
    h.state(AgentState::Working, 4)?;
    let before = h.status(AGENT)?;

    // When: the program prints text that looks like status directives.
    let fake =
        b"idle\r\nstate=done\r\n\xe2\x9c\x93 Done. Waiting for input >\r\n7501;state=idle\r\n";
    let outcomes = ingest(&h, fake)?;

    // Then: no OSC record exists and the report stands unchanged.
    assert_eq!(outcomes, []);
    assert_eq!(h.status(AGENT)?, before);
    Ok(())
}

#[test]
fn an_osc_report_takes_the_next_sequence_and_keeps_raw_and_decoding_provenance() -> TestResult {
    let h = harness()?;
    h.state(AgentState::Idle, 4)?;
    let payload = "state=working:app=custom:title=aGVsbG8=:msg=%%not-base64%%";
    let outcomes = ingest(&h, &osc(payload))?;
    assert_eq!(outcomes, [Ingested::Applied]);
    let status = h.status(AGENT)?.status;
    let root = status.root.ok_or("root")?;
    assert_eq!(root.origin, OpenEnum::Known(StatusOrigin::Osc));
    assert_eq!(root.report_seq, ReportSeq::new(5));
    assert_eq!(root.raw.as_deref(), Some(payload));
    assert_eq!(root.decode_errors, ["msg"]);
    assert_eq!(root.report.title.as_deref(), Some("hello"));
    assert_eq!(root.report.msg, None);
    let provenance = root.osc.ok_or("provenance")?;
    assert_eq!(provenance.fields.msg_b64.as_deref(), Some("%%not-base64%%"));
    assert_eq!(provenance.decoded.msg, None);
    assert_eq!(
        provenance.decode_errors,
        [OscDecodeError::new(
            OscTextField::Msg,
            DecodeFailure::InvalidBase64
        )]
    );
    let stale = h.state(AgentState::Done, 5)?;
    assert_eq!(kind(&stale), Some("stale_seq"));
    Ok(())
}

#[test]
fn base64_of_non_utf8_text_is_an_invalid_utf8_error_without_decoded_text() -> TestResult {
    let h = harness()?;
    ingest(&h, &osc("state=blocked:msg=/w=="))?;
    let root = h.status(AGENT)?.status.root.ok_or("root")?;
    let provenance = root.osc.ok_or("provenance")?;
    assert_eq!(
        provenance.decode_errors,
        [OscDecodeError::new(
            OscTextField::Msg,
            DecodeFailure::InvalidUtf8
        )]
    );
    assert_eq!(root.report.msg, None);
    Ok(())
}

#[test]
fn oversized_and_malformed_records_are_counted_and_change_nothing() -> TestResult {
    let h = harness()?;
    h.state(AgentState::Working, 4)?;
    let before = h.status(AGENT)?;
    let oversized = osc(&format!("state=idle:msg={}", "A".repeat(5000)));
    let mut unterminated = b"\x1b]7501;state=idle:msg=".to_vec();
    // The engine cuts an unterminated sequence at 64 KiB and reports it oversize.
    unterminated.extend(std::iter::repeat_n(b'B', 70_000));
    let malformed = [
        osc("state=thinking"),
        osc("state=idle:colour=red"),
        osc("msg=aGk="),
        osc("state=idle:state=done"),
        osc("state=idle:progress=101"),
    ];
    let mut outcomes = ingest(&h, &oversized)?;
    outcomes.extend(ingest(&h, &unterminated)?);
    for record in &malformed {
        outcomes.extend(ingest(&h, record)?);
    }
    assert!(outcomes.iter().all(|outcome| *outcome == Ingested::Dropped));
    assert_eq!(outcomes.len(), 7);
    let after = h.status(AGENT)?;
    assert_eq!(after.status, before.status);
    assert_eq!(after.ingest, IngestCounters::new(2, 5, 0));
    Ok(())
}

#[test]
fn a_query_and_other_osc_numbers_are_ignored() -> TestResult {
    let h = harness()?;
    let outcomes = ingest(
        &h,
        b"\x1b]7501;?\x1b\\\x1b]9;note\x07\x1b]7502;state=idle\x07",
    )?;
    assert_eq!(
        outcomes,
        [Ingested::Ignored, Ingested::Ignored, Ingested::Ignored]
    );
    assert!(!h.status(AGENT)?.status.reported);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn every_split_of_the_output_yields_the_same_status_record(
        cuts in proptest::collection::vec(0usize..120, 0..6),
    ) {
        let mut bytes = b"text before ".to_vec();
        bytes.extend(osc("state=working:id=sub:kind=tool:progress=40:title=dGl0bGU=:msg=bXNn"));
        bytes.extend_from_slice(b" after");
        let mut points: Vec<usize> = cuts.into_iter().map(|cut| cut.min(bytes.len())).collect();
        points.sort_unstable();
        let mut chunks = Vec::new();
        let mut from = 0;
        for point in points.into_iter().chain([bytes.len()]) {
            chunks.push(bytes.get(from..point).unwrap_or_default());
            from = point;
        }
        let whole = records(&[&bytes]).map_err(|error| TestCaseError::fail(error.to_string()))?;
        let split = records(&chunks).map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(whole.len(), 1);
        prop_assert_eq!(split.len(), 1);
        let (Some(whole), Some(split)) = (whole.first(), split.first()) else {
            return Err(TestCaseError::fail("both reads located one record"));
        };
        let (OscOutcome::Report(whole), OscOutcome::Report(split)) = (classify(whole), classify(split))
        else {
            return Err(TestCaseError::fail("both reads are status reports"));
        };
        prop_assert_eq!(&whole.report, &split.report);
        prop_assert_eq!(&whole.provenance, &split.provenance);
        prop_assert_eq!(whole.report.progress, Some(40));
        prop_assert_eq!(whole.report.msg.as_deref(), Some("msg"));
    }
}
