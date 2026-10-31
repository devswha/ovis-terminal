//! Who may write: the lease, the write grant and the raw-input grant at admission.

use ovis_terminal_core::input::encoding::InputKind;
use ovis_terminal_core::input::sequencer::{Payload, QueueLimits};
use ovis_terminal_protocol::schema::methods::input::Submit;

use super::TestResult;
use super::fixture::{Fixture, OBSERVER, RAW, WRITER, admit, modes, submission, view};

#[test]
fn lease_and_grant_rules_hold_at_admission() -> TestResult {
    // Given an operator holding the interact lease, a rival writer and an observer.
    let mut fixture = Fixture::new()?;
    let (mut leases, modes) = (fixture.leases()?, modes(b"")?);
    let operator = fixture.client("operator", WRITER)?;
    let rival = fixture.client("rival", WRITER)?;
    let watcher = fixture.client("watcher", OBSERVER)?;
    let unleased = fixture.leases()?;
    let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;
    let payload = Payload::Text {
        text: "x",
        submit: &Submit::None,
    };
    let observer_view = view(&unleased, &modes);
    let refused = admit(
        &mut sequencer,
        &observer_view,
        (
            watcher.writer(),
            submission("op", fixture.expected()?, payload),
        ),
    );
    assert_eq!(
        refused.map_err(|error| error.kind()).err(),
        Some("forbidden")
    );
    let lease = fixture.interact(&mut leases, &operator)?;
    let view = view(&leases, &modes);

    // When each writes, with and without the lease.
    let attempts = [
        (rival.writer(), Err("no_lease")),
        (rival.writer().presenting(lease), Err("no_lease")),
        (operator.writer(), Err("no_lease")),
        (operator.writer().presenting(lease), Ok(())),
    ];
    for (writer, expected) in attempts {
        let outcome = admit(
            &mut sequencer,
            &view,
            (writer, submission("op", fixture.expected()?, payload)),
        );
        // Then only the current holder presenting its lease is admitted.
        assert_eq!(outcome.map(|_| ()).map_err(|error| error.kind()), expected);
    }
    Ok(())
}

#[test]
fn raw_bytes_need_the_raw_grant_and_pass_unchanged() -> TestResult {
    // Given a plain writer and a writer holding the raw-input grant.
    let fixture = Fixture::new()?;
    let (leases, modes) = (fixture.leases()?, modes(b"")?);
    let view = view(&leases, &modes);
    let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;
    let plain = fixture.client("plain", WRITER)?;
    let operator = fixture.client("operator", RAW)?;
    let bytes = b"\x1b[999;unknown\x07\xff";

    // When both send raw bytes, including a sequence no encoder knows.
    let refused = admit(
        &mut sequencer,
        &view,
        (
            plain.writer(),
            submission("raw-1", fixture.expected()?, Payload::Raw(bytes)),
        ),
    );
    let receipt = admit(
        &mut sequencer,
        &view,
        (
            operator.writer(),
            submission("raw-2", fixture.expected()?, Payload::Raw(bytes)),
        ),
    )?;

    // Then only the grant holder is admitted and no byte is dropped or rewritten.
    assert_eq!(
        refused.map_err(|error| error.kind()).err(),
        Some("raw_not_authorized")
    );
    assert_eq!(
        (receipt.kind, receipt.admitted_bytes.get()),
        (InputKind::Raw, 15)
    );
    assert_eq!(
        sequencer.next(&view)?.dispatch.ok_or("no write")?.bytes,
        bytes
    );
    Ok(())
}
