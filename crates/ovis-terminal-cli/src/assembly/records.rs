//! The published terminal record of a lifecycle record, checked against the closed wire
//! type before it leaves the server.

use ovis_terminal_core::registry::{ResourceName, TerminalState as CoreState};
use ovis_terminal_core::terminals::Record;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::records::TerminalRecord;
use serde_json::{Value, json};

use super::actor::Runtime;
use super::error::refusal;

const fn state(record: &Record) -> &'static str {
    match record.lifecycle.state() {
        CoreState::Running => "live",
        CoreState::Finished if record.launch.keep => "lost",
        CoreState::Finished => "finished",
        _ => "lost",
    }
}

fn finished(record: &Record) -> Value {
    let Some(exit) = &record.exit else {
        return Value::Null;
    };
    let reason = match (exit.code, exit.signal) {
        (Some(_), _) => "exited",
        (None, Some(_)) => "signaled",
        (None, None) => "holder_lost",
    };
    json!({
        "reason": reason,
        "exit_code": exit.code,
        "signal": exit.signal,
        "finished_ms": "0",
        "final_output_offset": exit.final_offset.to_string(),
    })
}

/// The status of a program that never reported one.
pub(super) fn unreported() -> Value {
    json!({
        "reported": false, "root": null, "records": {}, "turn": "0",
        "process_incarnation": null, "report_seq": null, "finish_generation": "0",
        "last_turn": null, "agent_id": null, "session": null,
    })
}

impl Runtime {
    fn process(&self, record: &Record) -> Value {
        let Some(process) = record.lifecycle.last_process() else {
            return Value::Null;
        };
        let started = self
            .live
            .get(&record.id())
            .map_or_else(|| "0".to_owned(), |live| live.spawned_ms.get().to_string());
        let program = serde_json::to_value(&record.launch.program).unwrap_or(Value::Null);
        json!({
            "incarnation": process.generation().get().to_string(),
            "pid": u32::try_from(process.pid().get()).ok(),
            "start_token": process.start_token().as_str(),
            "argv": program.get("argv"),
            "cwd": program.get("cwd"),
            "started_ms": started,
        })
    }

    fn geometry(&self, record: &Record) -> Value {
        let live = self.live.get(&record.id());
        let size = live
            .and_then(|live| live.hub.screen().first_paint().ok())
            .map_or_else(
                || json!({"cols": record.launch.geometry.cols, "rows": record.launch.geometry.rows}),
                |paint| json!({"cols": paint.geometry.columns(), "rows": paint.geometry.rows()}),
            );
        let owner = live
            .and_then(|live| live.hub.with_leases(|leases| leases.view().geometry).ok())
            .flatten()
            .map(|lease| lease.lease_id.to_string());
        json!({"size": size, "owner_lease": owner, "revision": record.revision})
    }

    /// The wire record of `record`, decoded as [`TerminalRecord`] before it is returned.
    pub(super) fn record_value(&self, record: &Record) -> Result<Value, BoundaryError> {
        let id = record.id();
        let address = id
            .to_string()
            .split_once(':')
            .map(|(_, number)| format!("t{number}"));
        let created = self.created.get(&id).copied().unwrap_or(record.revision);
        let value = json!({
            "terminal_id": id.to_string(),
            "address": address,
            "name": record.launch.name.as_ref().map(ResourceName::as_str),
            "labels": record.launch.labels,
            "machine_id": self.identity.machine_id().to_string(),
            "state": state(record),
            "origin": record.origin,
            "terminal_incarnation": record.lifecycle.generation().get().to_string(),
            "process": self.process(record),
            "keep": record.launch.keep,
            "parent": null,
            "notify": false,
            "die_with_parent": false,
            "geometry": self.geometry(record),
            "output_offset": record.output_offset,
            "revision": record.revision,
            "status": unreported(),
            "agent": null,
            "finished": finished(record),
            "created_seq": created,
            "updated_seq": record.revision,
        });
        let checked: TerminalRecord = serde_json::from_value(value)
            .map_err(|error| super::error::internal("terminal record schema")(&error))?;
        serde_json::to_value(checked).map_err(|_| refusal("internal", "report the build"))
    }
}
