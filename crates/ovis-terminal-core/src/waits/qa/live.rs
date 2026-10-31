//! Send-then-wait and interrupt against a real program on a holder PTY.

use std::io::Write as _;
use std::time::{Duration, Instant};

use ovis_terminal_core::input::receipts::{Delivery, Receipt};
use ovis_terminal_core::input::sequencer::{Moment, Payload, Sequencer, View};
use ovis_terminal_core::leases::TerminalLeases;
use ovis_terminal_core::waits::{
    CancelScope, InterruptReport, OutputOutcome, OutputWait, ProcessObservation, ProcessState,
    StopEvidence, StopWatch, WatchEnd,
};
use ovis_terminal_protocol::schema::methods::input::Submit;
use ovis_terminal_protocol::schema::scalar::ByteOffset;

use super::TestResult;
use super::pty::Live;
use super::sequencer::{Fixture, NOW, WRITER, admit, modes, submission};

/// How long an interrupt is observed before its outcome is reported.
const OBSERVE: Duration = Duration::from_millis(300);

fn emit(label: &str, value: &impl serde::Serialize) -> TestResult {
    writeln!(
        std::io::stdout().lock(),
        "{label} {}",
        serde_json::to_string(value)?
    )?;
    Ok(())
}

/// Admits `payload` against the hub's current output offset and writes it to the PTY.
fn send(live: &Live, payload: Payload<'_>, op: &str) -> TestResult<(Receipt, Sequencer)> {
    let fixture = Fixture::new()?;
    let (leases, modes): (TerminalLeases, _) = (fixture.leases()?, modes(b"")?);
    let cursor = ByteOffset::new(live.waits.cursor()?.get());
    let view = View::new(&leases, &modes, Moment::new(NOW, cursor));
    let mut sequencer = live.sequencer(&fixture)?;
    let operator = fixture.client("operator", WRITER)?;
    let receipt = admit(
        &mut sequencer,
        &view,
        (
            operator.writer(),
            submission(op, fixture.expected()?, payload),
        ),
    )?;
    let written = live.drain(&mut sequencer, &view)?;
    let receipt = written
        .into_iter()
        .find(|written| written.op_id == receipt.op_id)
        .ok_or("no write evidence")?;
    Ok((receipt, sequencer))
}

/// `READY_MARK` echoed while the send was in flight is matched from the receipt's cursor.
///
/// # Errors
///
/// Fails when the holder, the sequencer or the hub fails.
///
/// # Panics
///
/// When an asserted outcome does not hold.
pub fn ready_mark_printed_during_send_is_matched_from_the_receipt_cursor() -> TestResult {
    let live = Live::start("stty raw -echo; printf ARMED; exec cat")?;
    let payload = Payload::Text {
        text: "READY_MARK",
        submit: &Submit::Enter,
    };
    let (receipt, _sequencer) = send(&live, payload, "send-ready-mark")?;
    let echoed = receipt.output_cursor.get().saturating_add(11);
    live.pump_until(|_| {
        live.waits
            .cursor()
            .is_ok_and(|cursor| cursor.get() >= echoed)
    })?;
    let after_send = live.waits.cursor()?;

    let from_receipt = live
        .waits
        .output(
            OutputWait::since(&receipt, b"READY_MARK")?,
            &CancelScope::new(),
        )?
        .wait(Instant::now())?;
    let from_now = live
        .waits
        .output(
            OutputWait::new(after_send, b"READY_MARK")?,
            &CancelScope::new(),
        )?
        .wait(Instant::now())?;

    let start = ovis_terminal_vt::OutputOffset::new(receipt.output_cursor.get());
    assert!(matches!(receipt.delivery, Delivery::PtyWritten { .. }));
    assert_eq!(
        from_receipt,
        OutputOutcome::Matched {
            at: start,
            cursor: ovis_terminal_vt::OutputOffset::new(start.get().saturating_add(10))
        }
    );
    assert_eq!(from_now, OutputOutcome::Timeout { cursor: after_send });
    emit("receipt", &receipt)?;
    emit("wait_from_receipt_cursor", &from_receipt)?;
    emit("wait_armed_after_send", &from_now)?;
    Ok(())
}

/// Escape to a program that keeps running is delivered, observed running, and not stopped.
///
/// # Errors
///
/// Fails when the holder, the sequencer or the hub fails.
///
/// # Panics
///
/// When an asserted outcome does not hold.
pub fn escape_to_a_running_program_is_delivered_but_not_stopped() -> TestResult {
    let live = Live::start("stty raw -echo; printf ARMED; exec sleep 30")?;
    let ticket = live.waits.stop(StopWatch::new(), &CancelScope::new())?;
    let keys = ["esc".to_owned()];
    let (receipt, _sequencer) = send(&live, Payload::Keys(&keys), "interrupt-esc")?;
    let deadline = Instant::now()
        .checked_add(OBSERVE)
        .ok_or("clock overflow")?;
    let report = InterruptReport::new(&receipt, ticket.wait(deadline)?);

    assert!(
        report.delivered,
        "the Escape byte reached the PTY: {receipt:?}"
    );
    assert_eq!(report.observed.end, WatchEnd::Timeout);
    assert_eq!(report.observed.process, ProcessObservation::Running);
    assert!(!report.stopped);
    assert!(live.os_lists_process()?, "ps still lists the program");
    emit("interrupt", &report)?;
    Ok(())
}

/// Ctrl+C that ends the program is stopped by the holder's exit evidence.
///
/// # Errors
///
/// Fails when the holder, the sequencer or the hub fails.
///
/// # Panics
///
/// When an asserted outcome does not hold.
pub fn interrupt_that_ends_the_program_is_stopped_by_exit_evidence() -> TestResult {
    let live = Live::start("printf ARMED; exec sleep 30")?;
    let ticket = live.waits.stop(StopWatch::new(), &CancelScope::new())?;
    let keys = ["ctrl+c".to_owned()];
    let (receipt, _sequencer) = send(&live, Payload::Keys(&keys), "interrupt-ctrl-c")?;
    live.pump_until(|_| {
        live.waits
            .process()
            .is_ok_and(|process| process != ProcessState::Running)
    })?;
    let report = InterruptReport::new(&receipt, ticket.wait(Instant::now())?);

    assert!(report.delivered);
    assert!(report.stopped);
    assert!(matches!(
        report.observed.end,
        WatchEnd::Stopped { evidence: StopEvidence::Exit { exit } } if exit.signal == Some(2)
    ));
    assert!(!live.os_lists_process()?, "ps no longer lists the program");
    emit("interrupt", &report)?;
    Ok(())
}
