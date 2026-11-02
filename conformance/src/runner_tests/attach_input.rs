//! Plan todo 30: attach, input and shared-storage conformance of the assembled binary.

use serde_json::Value;

use super::support::{Ran, TestResult, assertion, conformance, field};

#[path = "attach_input/oracle.rs"]
mod oracle;

/// The value the binary answered for one named check, which must have held.
fn held<'a>(ran: &'a Ran, check: &str) -> TestResult<&'a Value> {
    let found =
        assertion(&ran.evidence, check).ok_or_else(|| format!("{check} was not checked"))?;
    assert_eq!(field(found, "/passed"), true, "{check}: {found}");
    Ok(field(found, "/actual"))
}

/// A canonical decimal string the binary answered for one named check.
fn decimal(ran: &Ran, check: &str) -> TestResult<u64> {
    Ok(held(ran, check)?
        .as_str()
        .ok_or("a decimal string")?
        .parse()?)
}

/// The QA output ring both cases serve with.
const RING: u64 = 65_536;

/// Runs one case and requires exit 0 with at least one assertion and none failed.
fn passed(scenario: &str, case: &str) -> TestResult<Ran> {
    let ran = conformance(scenario, case, &[])?;
    assert_eq!(ran.output.status.code(), Some(0), "{}", ran.explain());
    assert_eq!(field(&ran.evidence, "/result"), "passed");
    assert_eq!(field(&ran.evidence, "/totals/failed"), "0");
    let assertions: u64 = field(&ran.evidence, "/totals/assertions")
        .as_str()
        .ok_or("assertion total")?
        .parse()?;
    assert!(assertions > 0, "{}", ran.explain());
    Ok(ran)
}

/// A split-write program's screen equals an independent VT fed its raw bytes at once; it
/// saw one query reply, and a late observer's paints plus the tail rebuild that screen.
fn reconstructs_from_raw_bytes(ran: &Ran) -> TestResult {
    let raw = oracle::bytes(ran, "reply[fx-raw]/result/data_b64")?;
    let replayed = oracle::screen(&raw)?;
    let shown: Vec<String> =
        serde_json::from_value(held(ran, "reply[fx-screen]/result/lines")?.clone())?;
    assert_eq!(
        replayed.lines, shown,
        "the daemon's screen equals a VT fed the raw bytes at once"
    );
    assert_eq!(
        held(ran, "reply[fx-screen]/result/cursor")?,
        &replayed.cursor
    );
    assert_eq!(
        shown.get(..3),
        Some(
            ["utf8:\u{20ac} csi:RED", "REPLY-ONCE", "NEXT[x]"]
                .map(String::from)
                .as_slice()
        ),
        "split UTF-8 and CSI rendered whole, one query reply, then the operator's byte"
    );
    assert_eq!(
        *held(ran, "reply[late]/result/first_paint/inexact")?,
        serde_json::json!([])
    );
    oracle::late_paint_and_tail_rebuild_the_screen(ran)
}

/// The QA ring overflowed into a floor at head - ring, a resynced observer within its credit
/// and exactly one accounted loss.
fn overflow_is_accounted(ran: &Ran) -> TestResult {
    let head = decimal(ran, "reply[screen]/result/output_offset")?;
    let floor = head.checked_sub(RING).ok_or("the head is past one ring")?;
    let newest_byte = head.checked_sub(1).ok_or("output was produced")?;
    assert_eq!(held(ran, "reply[evicted]/result/gap")?, true);
    assert_eq!(decimal(ran, "reply[evicted]/result/floor_offset")?, floor);
    assert_eq!(decimal(ran, "reply[evicted]/result/from_offset")?, floor);
    assert_eq!(held(ran, "reply[floor-read]/result/gap")?, false);
    let resynced = decimal(ran, "reply[repaint]/result/stream_offset")?;
    assert_eq!(
        decimal(ran, "reply[repaint]/result/checkpoint/stream_offset")?,
        resynced
    );
    assert!(
        resynced <= decimal(ran, "hello/result/limits/credit_initial_bytes")?,
        "a viewer that never credits is never sent more than its initial credit"
    );
    let lost = |end: &str| {
        decimal(
            ran,
            &format!("reply[metrics]/result/consumers/0/losses/0/range/{end}"),
        )
    };
    assert_eq!(
        lost("first_offset")?,
        decimal(
            ran,
            "notification[attach.control]/params/skipped/from_output_offset"
        )?,
        "the loss starts where the observer's gap record says its skip did"
    );
    assert_eq!(
        lost("last_offset")?,
        newest_byte,
        "and ends at the repaint head"
    );
    assert_eq!(
        *held(ran, "reply[metrics]/result/consumers/1/losses")?,
        serde_json::json!([])
    );
    Ok(())
}

/// Both viewers get one first paint at the prompt's end, input is admitted there before
/// its output, the marker wait matches exactly one echoed command line later, the
/// operator takes the lease over by epoch, the ring overflows into an accounted gap, a
/// split-write program's screen equals an independent VT fed its raw bytes. Offsets are
/// relative to the run's own prompt: a program may print invisible bytes first (Darwin's
/// bash sends the 8-byte `ESC[?1034h`).
#[test]
fn happy() -> TestResult {
    let ran = passed("part-a/task-30", "happy")?;
    let paint = held(&ran, "reply[writer-open]/result/first_paint")?;
    assert_eq!(
        held(&ran, "reply[observer-open]/result/first_paint")?,
        paint
    );
    let prompt_end = held(&ran, "reply[prompt]/result/cursor/offset")?;
    assert_eq!(field(paint, "/output_offset"), prompt_end);
    assert_eq!(held(&ran, "reply[prompt-bytes]/result/data_b64")?, "JCA=");
    assert_eq!(
        held(&ran, "reply[mark]/result/receipt/output_cursor/offset")?,
        prompt_end
    );
    let echoed = u64::try_from("echo mark-$((6*7))\r\n".len())?;
    assert_eq!(
        decimal(&ran, "reply[mark-wait]/result/matched_at")?,
        decimal(&ran, "reply[prompt]/result/cursor/offset")? + echoed
    );
    assert_eq!(
        held(&ran, "reply[mark-bytes]/result/data_b64")?,
        "bWFyay00Mg=="
    );
    assert_eq!(held(&ran, "reply[mark-barrier]/result/reached")?, true);
    assert_eq!(
        held(&ran, "reply[operator-open]/result/retry/on")?,
        "lease_released"
    );
    assert_eq!(
        held(&ran, "reply[takeover]/result/displaced/holder")?,
        "writer"
    );
    assert_eq!(
        held(&ran, "notification[attach.control]/params/close/reason")?,
        "taken_over"
    );
    assert_eq!(
        held(&ran, "reply[repaint]/result/checkpoint/output_offset")?,
        held(&ran, "reply[screen]/result/output_offset")?
    );
    reconstructs_from_raw_bytes(&ran)?;
    oracle::repaint_after_overflow_and_tail_rebuild_the_screen(&ran)?;
    overflow_is_accounted(&ran)?;
    Ok(())
}

/// Each adversarial request is answered by its typed rejection, and the stalled child
/// keeps its queued order and its process after an Escape that does nothing.
#[test]
fn failure() -> TestResult {
    let ran = passed("part-a/task-30", "failure")?;
    for (check, kind) in [
        ("reply[observer-send]/error/data/kind", "forbidden"),
        ("reply[credit-max]/error/data/kind", "invalid_credit"),
        ("reply[credit-numeric]/error/data/kind", "invalid_params"),
        ("reply[credit-stale]/error/data/kind", "stream_not_found"),
        (
            "reply[stale-takeover]/error/data/kind",
            "lease_epoch_mismatch",
        ),
        ("reply[revoked]/error/data/kind", "no_lease"),
        ("reply[paste]/error/data/kind", "paste_refused"),
        ("reply[huge]/error/data/kind", "too_large"),
        ("reply[no-spawn]/error/data/kind", "not_found"),
        ("reply[fill6]/error/data/kind", "input_overflow"),
    ] {
        assert_eq!(held(&ran, check)?, kind, "{check}");
    }
    assert_eq!(
        held(&ran, "reply[after-fake]/result/status_at_send/reported")?,
        false
    );
    assert_eq!(held(&ran, "reply[barrier]/result/reached")?, false);
    assert_eq!(held(&ran, "reply[escape]/result/receipt/queued_seq")?, "3");
    assert_eq!(held(&ran, "reply[still]/result/outcome")?, "timeout");
    assert_eq!(held(&ran, "reply[alive]/result/finished")?, &Value::Null);
    assert!(
        decimal(&ran, "reply[after-cap]/result/pending_bytes")?
            <= decimal(&ran, "hello/result/limits/input_queue_bytes")?,
        "the refused write kept the queue inside its advertised budget"
    );
    assert_eq!(
        *held(&ran, "reply[mid]/result/first_paint/inexact")?,
        serde_json::json!(["control_inside_open_sequence"])
    );
    assert_eq!(
        *held(&ran, "reply[mid-repaint]/result/checkpoint/inexact")?,
        serde_json::json!([])
    );
    for (writer, seq) in [("a1", "1"), ("b1", "2"), ("a2", "3"), ("b2", "4")] {
        let check = format!("reply[{writer}]/result/receipt/queued_seq");
        assert_eq!(held(&ran, &check)?, seq, "{check}");
    }
    assert_eq!(
        held(&ran, "reply[delivered]/result/data_b64")?,
        "b25lLWENCnR3by1hDQpvbmUtYg0KdHdvLWINCg=="
    );
    let armed = decimal(&ran, "reply[head-now]/result/output_offset")?;
    assert!(decimal(&ran, "reply[overtaken]/result/matched_at")? > armed + RING);
    assert_eq!(
        held(&ran, "reply[expired]/error/data/kind")?,
        "cursor_expired"
    );
    assert_eq!(held(&ran, "reply[evicted]/result/gap")?, true);
    assert_eq!(
        decimal(&ran, "reply[evicted]/result/from_offset")?,
        decimal(&ran, "reply[evicted]/result/floor_offset")?
    );
    Ok(())
}
