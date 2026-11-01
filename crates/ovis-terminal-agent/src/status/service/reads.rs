//! Reads that answer one terminal's turn record and agent binding.

use ovis_terminal_core::ops::{Completion, OperationError};
use ovis_terminal_core::registry::TerminalId;
use ovis_terminal_protocol::schema::methods::status::TurnRecord;
use ovis_terminal_protocol::schema::records::AgentBinding;
use ovis_terminal_store::Snapshot;

use super::super::{AgentStatusEvent, StatusError};
use super::{StatusService, settle};

impl StatusService {
    /// Reads a terminal's current turn: its record once it ended, otherwise the turn that
    /// is still open; a read never changes turn or finish facts.
    ///
    /// # Errors
    ///
    /// Returns a store or decoding failure.
    pub fn turn<D: From<AgentStatusEvent>>(
        &self,
        snapshot: &Snapshot,
        terminal: TerminalId,
    ) -> Result<Completion<D>, OperationError> {
        settle(self.current(snapshot, terminal).and_then(|stored| {
            let status = &stored.row.status;
            let record: TurnRecord = match &status.last_turn {
                Some(last) if last.turn == status.turn => last.clone(),
                _ => serde_json::from_value(serde_json::json!({
                    "turn": status.turn, "outcome": null, "started_seq": null,
                    "ended_seq": null, "send_op": null, "finish_generation": null,
                    "ended_on": null, "process_incarnation": status.process_incarnation,
                    "agent_id": null, "session": null,
                }))?,
            };
            Ok(Completion::new(serde_json::to_value(record)?))
        }))
    }

    /// The agent bound to `terminal` at `snapshot`, if any.
    ///
    /// # Errors
    ///
    /// Returns a store or decoding failure.
    pub fn binding(
        &self,
        snapshot: &Snapshot,
        terminal: TerminalId,
    ) -> Result<Option<AgentBinding>, StatusError> {
        self.current(snapshot, terminal)
            .map(|stored| stored.row.agent)
    }
}
