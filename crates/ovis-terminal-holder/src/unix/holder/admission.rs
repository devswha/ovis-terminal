//! Admission of one input request into a terminal's bounded queue.

use std::sync::Arc;

use super::state::{HolderState, WriteAck, byte_count};
use crate::protocol::{Input, InputRejected, InputRejection, Message};
use crate::unix::error::HolderError;
use crate::unix::input::{PushRejected, QueuedInput};

/// Queues `input`, acknowledging fenced entries unwritten and publishing a typed
/// `InputRejected` when either queue bound would be exceeded.
pub(super) fn admit_input(state: &mut HolderState, input: Input) -> Result<(), HolderError> {
    let terminal = state.terminal_mut(&input.target)?;
    if terminal.held.process_incarnation != Some(input.grant.process_incarnation) {
        return Err(HolderError::StaleTarget {
            terminal_id: input.target.terminal_id,
        });
    }
    if let Some(last) = terminal
        .admitted_sequence
        .filter(|last| input.sequence <= *last)
    {
        return Err(HolderError::SequenceRegression {
            sequence: input.sequence,
            last,
        });
    }
    let live = terminal.live.as_ref().ok_or(HolderError::ProcessExited)?;
    let queue = Arc::clone(&live.input);
    let output_offset = terminal.tail.head();
    let Input {
        target,
        sequence,
        grant,
        bytes,
    } = input;
    let requested_bytes = bytes.len();
    let admission = queue.push(QueuedInput {
        sequence,
        grant,
        bytes,
    });
    let fence_epoch = queue.fence_epoch();
    for fenced in admission.fenced {
        let ack = WriteAck {
            input: fenced,
            written: 0,
            fence_epoch,
            output_offset,
        };
        state.acknowledge(&target.terminal_id, &ack);
    }
    match admission.outcome {
        Ok(()) => {
            state.terminal_mut(&target)?.admitted_sequence = Some(sequence);
            Ok(())
        }
        Err(PushRejected::Closed) => Err(HolderError::ProcessExited),
        Err(PushRejected::Full { queued, input }) => {
            let limits = queue.limits();
            let error = HolderError::InputOverflow {
                queued_bytes: queued.bytes,
                queued_entries: queued.entries,
                requested_bytes,
                limits,
            };
            state.publish(Message::InputRejected(InputRejected {
                target,
                sequence: input.sequence,
                reason: InputRejection::InputOverflow,
                hint: error.hint().to_owned(),
                queued_bytes: byte_count(queued.bytes),
                queued_entries: byte_count(queued.entries),
                limit_bytes: byte_count(limits.bytes),
                limit_entries: byte_count(limits.entries),
            }));
            Err(error)
        }
    }
}
