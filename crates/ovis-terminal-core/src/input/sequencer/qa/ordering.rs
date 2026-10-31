//! Two writers on a real PTY: whole submissions, admission order, admission before write.

use std::io::Write as _;
use std::time::{Duration, Instant};

use ovis_terminal_core::input::receipts::{Delivery, InputSeq};
use ovis_terminal_core::input::sequencer::{BarrierState, Payload};
use ovis_terminal_protocol::schema::methods::input::Submit;

use super::TestResult;
use super::fixture::{Fixture, WRITER, admit, modes, submission, view};
use super::pty::{Pty, ready};

/// Two writers' interleaved submissions reach the program whole and in admission order.
///
/// # Errors
///
/// Fails when the holder or the sequencer fails.
///
/// # Panics
///
/// When an asserted ordering, receipt or byte count does not hold.
pub fn two_writers_keep_submissions_contiguous_in_admission_order() -> TestResult {
    // Given a raw-mode program echoing its input byte for byte, and two writers.
    let mut pty = Pty::start("stty raw -echo; printf ARMED; exec cat")?;
    let fixture = Fixture::new()?;
    let (leases, modes) = (fixture.leases()?, modes(b"")?);
    let view = view(&leases, &modes);
    let mut sequencer = ready(&fixture, &pty)?;
    let (alice, bob) = (
        fixture.client("alice", WRITER)?,
        fixture.client("bob", WRITER)?,
    );

    // When their submissions interleave, each admitted before any PTY write happens.
    let mut receipts = Vec::new();
    for (index, writer) in [&alice, &bob, &alice, &bob].into_iter().enumerate() {
        let text = format!(
            "{}-submission-{index}",
            if index % 2 == 0 { "alpha" } else { "beta" }
        );
        let payload = Payload::Text {
            text: &text,
            submit: &Submit::Enter,
        };
        let started = Instant::now();
        let receipt = admit(
            &mut sequencer,
            &view,
            (
                writer.writer(),
                submission(&text, fixture.expected()?, payload),
            ),
        )?;
        assert!(started.elapsed() < Duration::from_millis(100));
        assert_eq!(receipt.delivery, Delivery::Queued);
        receipts.push((text, receipt));
    }
    assert!(pty.output.is_empty(), "admission wrote nothing");
    assert!(matches!(
        sequencer.barrier(InputSeq::new(2))?,
        BarrierState::Pending { .. }
    ));
    let written = pty.drain(&mut sequencer, &view)?;
    let expected: Vec<u8> = receipts
        .iter()
        .flat_map(|(text, _)| format!("{text}\r").into_bytes())
        .collect();
    pty.until(|pty, _| pty.output.len() >= expected.len())?;

    // Then the program received every submission whole, in admission order.
    assert_eq!(pty.output, expected);
    assert_eq!(sequencer.barrier(InputSeq::new(4))?, BarrierState::Reached);
    for (receipt, (_, admitted)) in written.iter().zip(&receipts) {
        assert_eq!(receipt.queued_seq, admitted.queued_seq);
        assert!(
            matches!(receipt.delivery, Delivery::PtyWritten { written_bytes, .. } if written_bytes == admitted.admitted_bytes)
        );
    }
    assert_eq!(written.len(), 4);
    let mut out = std::io::stdout().lock();
    for receipt in &written {
        writeln!(out, "receipt {}", serde_json::to_string(receipt)?)?;
    }
    let hex = pty
        .output
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .concat();
    writeln!(out, "pty_bytes_hex {hex}")?;
    Ok(())
}
