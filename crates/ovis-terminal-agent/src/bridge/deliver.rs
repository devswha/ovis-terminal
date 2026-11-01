//! Extension events derived from committed answer receipts, and their acknowledgements.
//!
//! A session takes answers in closing-sequence order and the program's watermark moves
//! past each one handed out, so an answer is handed out at most once per program.

use ovis_terminal_protocol::domain::prompt_bridge::{
    DeliveryRecord, DeliveryState, ExtensionEvent,
};
use ovis_terminal_protocol::schema::ids::{PromptId, TerminalId as WireTerminal};
use ovis_terminal_protocol::schema::methods::status::PromptState;
use ovis_terminal_protocol::schema::scalar::RecordVersion;
use ovis_terminal_store::Snapshot;

use super::session::Program;
use super::{BridgeError, BridgeToken, PromptBridge};

impl PromptBridge {
    /// Hands the session its program's next extension answers, oldest first, up to the
    /// room left in its queue.
    ///
    /// # Errors
    ///
    /// Returns `forbidden` for a session that is not current and `limit_exceeded` while
    /// the queue is full; a store or decoding failure is returned as the prompt error.
    pub fn next(
        &self,
        snapshot: &Snapshot,
        token: &BridgeToken,
    ) -> Result<Vec<ExtensionEvent>, BridgeError> {
        let mut programs = self.lock()?;
        let program = self.session(&mut programs, token)?;
        let room = usize::from(self.limit.get()).saturating_sub(program.in_flight.len());
        if room == 0 {
            return Err(BridgeError::QueueFull {
                limit: self.queue_limit(),
            });
        }
        let mut events = self.undelivered(snapshot, program)?;
        events.truncate(room);
        if let Some(last) = events.last() {
            program.watermark = Some(last.answered_seq);
        }
        program.in_flight.extend(events.iter().cloned());
        drop(programs);
        Ok(events)
    }

    fn undelivered(
        &self,
        snapshot: &Snapshot,
        program: &Program,
    ) -> Result<Vec<ExtensionEvent>, BridgeError> {
        let answered = [PromptState::Answered];
        let filter = (Some(program.owner.terminal), Some(&answered[..]));
        let records = self
            .prompts
            .list(snapshot, filter)
            .map_err(BridgeError::Prompt)?;
        let mut events: Vec<ExtensionEvent> = records
            .iter()
            .filter(|record| record.process_incarnation == program.owner.process)
            .filter_map(ExtensionEvent::from_answered)
            .filter(|event| {
                program
                    .watermark
                    .is_none_or(|seen| event.answered_seq > seen)
            })
            .collect();
        events.sort_by_key(|event| event.answered_seq);
        Ok(events)
    }

    /// Acknowledges the delivered answer of `prompt` at `version`, the version its
    /// receipt committed.
    ///
    /// # Errors
    ///
    /// Returns `forbidden`, `not_delivered`, or `version_conflict` with the delivered
    /// version, also when the committed receipt no longer matches the delivery.
    pub fn ack(
        &self,
        snapshot: &Snapshot,
        (token, prompt, version): (&BridgeToken, &PromptId, RecordVersion),
    ) -> Result<DeliveryRecord, BridgeError> {
        let mut programs = self.lock()?;
        let program = self.session(&mut programs, token)?;
        let index = program
            .in_flight
            .iter()
            .position(|event| &event.prompt_id == prompt)
            .ok_or(BridgeError::NotDelivered)?;
        let delivered = program
            .in_flight
            .get(index)
            .ok_or(BridgeError::NotDelivered)?;
        if delivered.version != version {
            return Err(BridgeError::VersionConflict {
                current: delivered.version,
            });
        }
        let record = self
            .prompts
            .get(snapshot, prompt)
            .map_err(BridgeError::Prompt)?;
        let committed = ExtensionEvent::from_answered(&record);
        if committed.as_ref() != Some(delivered) {
            return Err(BridgeError::VersionConflict {
                current: record.version,
            });
        }
        let event = program.in_flight.remove(index);
        let acknowledged = DeliveryRecord::new(&event, DeliveryState::Acknowledged);
        program.settle(event.prompt_id, DeliveryState::Acknowledged);
        drop(programs);
        Ok(acknowledged)
    }

    /// Where the extension answer of `prompt` stands.
    ///
    /// # Errors
    ///
    /// Returns `not_found` for a prompt not answered through an extension, or the
    /// prompt's read failure.
    pub fn delivery(
        &self,
        snapshot: &Snapshot,
        prompt: &PromptId,
    ) -> Result<DeliveryRecord, BridgeError> {
        let record = self
            .prompts
            .get(snapshot, prompt)
            .map_err(BridgeError::Prompt)?;
        let event = ExtensionEvent::from_answered(&record).ok_or(BridgeError::NotExtension)?;
        let programs = self.lock()?;
        let program = programs.values().find(|program| {
            program.owner.process == event.process_incarnation
                && WireTerminal::new(program.owner.terminal.to_string()) == record.terminal_id
        });
        let state = program.map_or(DeliveryState::Pending, |program| state(program, &event));
        drop(programs);
        Ok(DeliveryRecord::new(&event, state))
    }
}

fn state(program: &Program, event: &ExtensionEvent) -> DeliveryState {
    if program.in_flight.contains(event) {
        return DeliveryState::Delivered;
    }
    let settled = program
        .settled
        .iter()
        .rev()
        .find(|(prompt, _)| *prompt == event.prompt_id);
    match settled {
        Some((_, state)) => *state,
        None if program
            .watermark
            .is_some_and(|seen| event.answered_seq <= seen) =>
        {
            DeliveryState::OutcomeUnknown
        }
        None => DeliveryState::Pending,
    }
}
