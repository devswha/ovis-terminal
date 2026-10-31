//! Revocation and takeover while entries are queued: no new byte reaches the program.

use std::io::Write as _;

use ovis_terminal_core::input::receipts::{Delivery, FenceReason};
use ovis_terminal_core::input::sequencer::Payload;
use ovis_terminal_core::leases::{ControlSlot, Takeover};
use ovis_terminal_holder::protocol::Message;
use ovis_terminal_protocol::schema::methods::input::Submit;

use super::TestResult;
use super::fixture::{Fixture, TAKEOVER, WRITER, admit, modes, submission, view};
use super::pty::{Pty, ready};

/// Takeover and credential revocation while entries are queued write no new byte.
///
/// # Errors
///
/// Fails when the holder or the sequencer fails.
///
/// # Panics
///
/// When an asserted ordering, receipt or byte count does not hold.
pub fn revocation_while_queued_writes_no_new_bytes() -> TestResult {
    // Given an operator's entries on an echoing program, the first one written.
    let mut pty = Pty::start("stty raw -echo; printf ARMED; exec cat")?;
    let mut fixture = Fixture::new()?;
    let (mut leases, modes) = (fixture.leases()?, modes(b"")?);
    let operator = fixture.client("operator", WRITER)?;
    let bridge = fixture.client("bridge", TAKEOVER)?;
    let lease = fixture.interact(&mut leases, &operator)?;
    let mut sequencer = ready(&fixture, &pty)?;
    for op in ["one", "two", "three"] {
        let payload = Payload::Text {
            text: op,
            submit: &Submit::None,
        };
        admit(
            &mut sequencer,
            &view(&leases, &modes),
            (
                operator.writer().presenting(lease),
                submission(op, fixture.expected()?, payload),
            ),
        )?;
    }
    let step = sequencer.next(&view(&leases, &modes))?;
    let first = step.dispatch.ok_or("no write")?;
    let sequence = first.sequence;
    pty.holder.input(first)?;
    let acked = pty.until(
        |_, last| matches!(last, Some(Message::InputWritten(ack)) if ack.sequence == sequence),
    )?;
    if let Some(Message::InputWritten(ack)) = acked.last() {
        sequencer.acknowledge(ack)?;
    }

    // When the bridge takes over and then its own credential is revoked mid-queue.
    let request =
        serde_json::from_value(serde_json::json!({"lease_epoch": "1", "reason": "bridge"}))?;
    let fresh = fixture.fresh()?;
    leases.take_over(
        bridge.caller(),
        &Takeover::new(ControlSlot::Interact, &request, fresh)?,
    )?;
    let fenced = sequencer.fence(&view(&leases, &modes))?;
    for op in ["four", "five"] {
        let payload = Payload::Text {
            text: op,
            submit: &Submit::None,
        };
        admit(
            &mut sequencer,
            &view(&leases, &modes),
            (
                bridge.writer().presenting(fresh),
                submission(op, fixture.expected()?, payload),
            ),
        )?;
    }
    let four = sequencer
        .next(&view(&leases, &modes))?
        .dispatch
        .ok_or("no write")?;
    fixture.revoke(&bridge)?;
    let four_sequence = four.sequence;
    pty.holder.input(four)?;
    let acked = pty.until(
        |_, last| matches!(last, Some(Message::InputWritten(ack)) if ack.sequence == four_sequence),
    )?;
    if let Some(Message::InputWritten(ack)) = acked.last() {
        sequencer.acknowledge(ack)?;
    }
    let rest = pty.drain(&mut sequencer, &view(&leases, &modes))?;
    pty.until(|pty, _| pty.output.len() >= 7)?;

    // Then only bytes written before each revocation reached the program.
    let reasons: Vec<_> = fenced
        .fenced
        .iter()
        .chain(&rest)
        .map(|receipt| (receipt.queued_seq.get(), receipt.delivery))
        .collect();
    let fence = |reason| Delivery::Fenced { reason };
    let expected = [
        (2, fence(FenceReason::LeaseRevoked)),
        (3, fence(FenceReason::LeaseRevoked)),
        (5, fence(FenceReason::CredentialRevoked)),
    ];
    assert_eq!(reasons, expected);
    assert_eq!(pty.output, b"onefour");
    let fenced = serde_json::to_string(&reasons)?;
    let bytes = String::from_utf8_lossy(&pty.output);
    writeln!(
        std::io::stdout().lock(),
        "fenced {fenced} pty_bytes {bytes}"
    )?;
    Ok(())
}
