//! The runtime side of T35's template launches. A planned launch runs through the same
//! `terminal.run` path as an argv launch: the holder assigns the program incarnation when it
//! spawns, so the daemon learns the terminal, its incarnation and its wait hub only once the
//! program is running (the seam the PR body names). The first message is written through
//! the same input admission as `input.raw`, presenting no lease.

use ovis_terminal_authz::Session;
use ovis_terminal_core::input::encoding::ModeSnapshot;
use ovis_terminal_core::input::sequencer::Payload;
use ovis_terminal_core::leases::{LeaseView, TerminalLeases};
use ovis_terminal_core::registry::{ProcessGeneration, ProcessIdentity, TerminalId};
use ovis_terminal_core::waits::Waits;
use ovis_terminal_protocol::errors::{BoundaryError, OpId};
use ovis_terminal_protocol::schema::methods::terminal::{LaunchProgram, TerminalRunParams};
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_services::templates::LaunchPlan;
use serde_json::{Value, json};

use super::actor::Runtime;
use super::error::refusal;
use super::input::Admission;

/// A planned launch the runtime started: its `terminal.run` result, the terminal, the
/// program incarnation the holder assigned and the incarnation's wait hub.
pub(super) struct Started {
    pub(super) result: Value,
    pub(super) terminal: TerminalId,
    pub(super) process: ProcessGeneration,
    pub(super) waits: Waits,
}

fn not_running() -> BoundaryError {
    refusal(
        "spawn_failed",
        "the template's program did not start; inspect terminal.get",
    )
}

impl Runtime {
    /// Runs `plan`'s literal argv and environment with the request's metadata.
    pub(super) fn run_planned(
        &mut self,
        session: &Session,
        (plan, params): (&LaunchPlan, &TerminalRunParams),
    ) -> Result<Started, BoundaryError> {
        let mut run = params.clone();
        run.template = None;
        run.first_message = None;
        run.program = Some(LaunchProgram::Argv(plan.argv.clone()));
        run.env = Some(plan.env.clone());
        let result = self.run_terminal(session, run)?;
        let terminal: TerminalId = result
            .pointer("/terminal/terminal_id")
            .and_then(Value::as_str)
            .and_then(|id| id.parse().ok())
            .ok_or_else(not_running)?;
        let record = self.lifecycle(|terminals| terminals.get(session, terminal))?;
        let process = record
            .lifecycle
            .process()
            .map(ProcessIdentity::generation)
            .ok_or_else(not_running)?;
        let waits = self
            .live
            .get(&terminal)
            .map(|live| (*live.waits).clone())
            .ok_or_else(not_running)?;
        Ok(Started {
            result,
            terminal,
            process,
            waits,
        })
    }

    /// The terminal's leases, input modes and running program incarnation now.
    pub(super) fn planned_boundary(
        &self,
        session: &Session,
        terminal: TerminalId,
    ) -> Result<(LeaseView, ModeSnapshot, ProcessGeneration), BoundaryError> {
        let live = self.live.get(&terminal).ok_or_else(not_running)?;
        let leases = live
            .hub
            .with_leases(TerminalLeases::view)
            .map_err(|_| not_running())?;
        let (modes, _) = live.view_parts().map_err(|_| not_running())?;
        let record = self.lifecycle(|terminals| terminals.get(session, terminal))?;
        let process = record
            .lifecycle
            .process()
            .map(ProcessIdentity::generation)
            .ok_or_else(not_running)?;
        Ok((leases, modes, process))
    }

    /// Writes one encoded submission presenting no lease and answers its input sequence.
    pub(super) fn write_planned(
        &mut self,
        session: &Session,
        (terminal, bytes): (TerminalId, &[u8]),
    ) -> Result<EventSeq, BoundaryError> {
        let target = serde_json::from_value(json!({"terminal_id": terminal.to_string()}))
            .map_err(|_| not_running())?;
        let op_id = OpId::new(uuid::Uuid::now_v7().to_string());
        let admission = Admission {
            target: &target,
            op_id: &op_id,
            lease: None,
            payload: Payload::Raw(bytes),
        };
        let receipt = self.admit_receipt(session, admission)?;
        Ok(EventSeq::new(receipt.queued_seq.get()))
    }
}
