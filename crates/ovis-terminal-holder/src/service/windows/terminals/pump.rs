//! One terminal's pump: output in order, then write acknowledgements, then the exit.
//!
//! The exit is published only after every byte through its final offset was published,
//! because [`ConptyTerminal::read`](crate::windows::ConptyTerminal::read) returns the
//! exit only to a reader already at that offset.

use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use super::{Entry, Table, lock};
use crate::protocol::{
    Exit, ExitUnknown, ExitUnknownReason, InputWritten, Message, Output, PtyIoFailure, PtyOperation,
};
use crate::windows::model::{OutputOffset, OutputRead};

/// How long one read waits for output before acknowledgements are checked again.
const WAIT: Duration = Duration::from_millis(50);

pub(super) fn start(table: Arc<Table>, entry: Arc<Entry>) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("ovis-holder-pump".to_owned())
        .spawn(move || run(&table, &entry))
}

/// Publishes `InputWritten` for every queued input the pseudoconsole has taken whole.
fn acknowledge(table: &Table, entry: &Entry) {
    let Some(written) = entry.terminal.input_progress().written else {
        return;
    };
    let mut acks = Vec::new();
    {
        let mut evidence = lock(&entry.evidence);
        while let Some(pending) = evidence.pending.pop_front() {
            if pending.queued > written {
                evidence.pending.push_front(pending);
                break;
            }
            evidence.input_sequence = evidence.input_sequence.max(pending.sequence);
            acks.push(InputWritten {
                target: entry.target.clone(),
                sequence: pending.sequence,
                grant_epoch: pending.grant_epoch,
                fence_epoch: pending.fence_epoch,
                ack_epoch: table.next_ack(),
                written_bytes: pending.bytes,
                output_offset: evidence.output,
            });
        }
        drop(evidence);
    }
    for ack in acks {
        table.publish(Message::InputWritten(ack));
    }
}

fn finish(table: &Table, entry: &Entry, exit: Exit) {
    acknowledge(table, entry);
    lock(&entry.evidence).exit = Some(exit.clone());
    table.publish(Message::Exit(exit));
}

fn run(table: &Table, entry: &Entry) {
    let mut cursor = OutputOffset::ZERO;
    loop {
        acknowledge(table, entry);
        match entry.terminal.read(cursor, WAIT) {
            Ok(OutputRead::Bytes { from, bytes, .. }) => {
                let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                cursor = OutputOffset::new(from.get().saturating_add(length));
                lock(&entry.evidence).output = cursor.get();
                table.publish(Message::Output(Output {
                    target: entry.target.clone(),
                    offset: from.get(),
                    bytes,
                    hangup: None,
                }));
            }
            Ok(OutputRead::Ended(ended)) => {
                return finish(table, entry, ended.exit_message(entry.target.clone()));
            }
            Ok(OutputRead::Pending) => {}
            Err(_) => {
                let lost = Exit {
                    target: entry.target.clone(),
                    code: None,
                    signal: None,
                    exit_unknown: Some(ExitUnknown {
                        reason: ExitUnknownReason::WaiterLost,
                        os_error: None,
                    }),
                    io_failure: Some(PtyIoFailure {
                        operation: PtyOperation::Pump,
                        os_error: None,
                        poll_revents: None,
                    }),
                    final_offset: cursor.get(),
                };
                return finish(table, entry, lost);
            }
        }
    }
}
