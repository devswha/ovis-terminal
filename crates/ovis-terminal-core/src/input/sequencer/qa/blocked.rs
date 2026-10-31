//! A program that never reads: admission and control stay prompt, the write ends exactly.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use ovis_terminal_core::input::receipts::{Delivery, FenceReason};
use ovis_terminal_core::input::sequencer::Payload;
use ovis_terminal_holder::protocol::{Message, Resize, Signal, SignalKind};
use ovis_terminal_protocol::schema::methods::input::Submit;

use super::TestResult;
use super::fixture::{Fixture, WRITER, admit, modes, submission, view};
use super::pty::{Pty, ready};

#[test]
fn a_reader_that_never_reads_blocks_only_its_own_entry() -> TestResult {
    // Given a raw-mode program that never reads and one large entry with the holder.
    let mut pty = Pty::start("stty raw -echo; printf ARMED; exec sleep 600")?;
    let fixture = Fixture::new()?;
    let (leases, modes) = (fixture.leases()?, modes(b"")?);
    let view = view(&leases, &modes);
    let sequencer = Mutex::new(ready(&fixture, &pty)?);
    let large = "a".repeat(1 << 20);
    let payload = Payload::Text {
        text: &large,
        submit: &Submit::None,
    };
    let writer = fixture.client("writer", WRITER)?;
    let mut guard = sequencer.lock().map_err(|_| "poisoned")?;
    admit(
        &mut guard,
        &view,
        (
            writer.writer(),
            submission("large", fixture.expected()?, payload),
        ),
    )?;
    let input = guard.next(&view)?.dispatch.ok_or("no write")?;
    let sequence = input.sequence;
    drop(guard);
    pty.holder.input(input)?;

    // When eight clients keep sending while the PTY write is stuck.
    let clients = (0..8)
        .map(|index| fixture.client(&format!("client-{index}"), WRITER))
        .collect::<TestResult<Vec<_>>>()?;
    let slowest = std::thread::scope(|scope| {
        let workers: Vec<_> = clients
            .iter()
            .enumerate()
            .map(|(index, client)| {
                let (sequencer, fixture) = (&sequencer, &fixture);
                scope.spawn(move || -> Result<Duration, String> {
                    let mut slowest = Duration::ZERO;
                    for round in 0..16 {
                        let op = format!("c{index}-{round}");
                        let payload = Payload::Text {
                            text: &op,
                            submit: &Submit::Enter,
                        };
                        let started = Instant::now();
                        let expected = fixture.expected().map_err(|error| error.to_string())?;
                        let mut sequencer = sequencer.lock().map_err(|_| "poisoned".to_owned())?;
                        let receipt = admit(
                            &mut sequencer,
                            &view,
                            (client.writer(), submission(&op, expected, payload)),
                        )
                        .map_err(|error| error.to_string())?;
                        drop(sequencer);
                        slowest = slowest.max(started.elapsed());
                        assert_eq!(receipt.delivery, Delivery::Queued);
                    }
                    Ok(slowest)
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().map_err(|_| "client panicked".to_owned())?)
            .collect::<Result<Vec<_>, String>>()
    })?;

    // Then every admission answered within 100 ms and the control lane still works.
    assert!(
        slowest
            .iter()
            .all(|elapsed| *elapsed < Duration::from_millis(100)),
        "{slowest:?}"
    );
    pty.holder.resize(&Resize {
        target: pty.target(),
        geometry: (100, 30),
        geometry_epoch: 1,
    })?;
    // And killing the program ends the write with the exact prefix the PTY took.
    pty.holder.signal(&Signal {
        target: pty.target(),
        signal: SignalKind::Kill,
    })?;
    let messages = pty.until(|_, last| matches!(last, Some(Message::Exit(_))))?;
    let mut sequencer = sequencer.into_inner().map_err(|_| "poisoned")?;
    let ack = messages
        .iter()
        .find_map(|message| match message {
            Message::InputWritten(ack) if ack.sequence == sequence => Some(ack),
            _ => None,
        })
        .ok_or("no acknowledgement")?;
    let receipt = sequencer.acknowledge(ack)?.ok_or("no receipt")?;
    let large_len = u64::try_from(large.len())?;
    assert!(
        matches!(receipt.delivery, Delivery::Partial { written_bytes, .. } if written_bytes.get() > 0 && written_bytes.get() < large_len),
        "{receipt:?}"
    );
    let ended = sequencer.exited();
    assert_eq!(ended.len(), 128);
    assert!(ended.iter().all(|receipt| receipt.delivery
        == Delivery::Fenced {
            reason: FenceReason::Finished
        }));
    Ok(())
}
