use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ovis_terminal_core::registry::{
    MachineId, ProcessHandle, ProcessPid, ProcessStartToken, TargetResolutionError,
    TerminalGeneration, TerminalId,
};
use ovis_terminal_core::terminals::process_info::{
    Liveness, NativeProcess, ProcessTable, Session, Terminal,
};
use ovis_terminal_core::terminals::{Field, Unavailable};
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::methods::terminal::TerminalProcessInfoParams;
use ovis_terminal_protocol::schema::records::ProcessRecord;
use ovis_terminal_protocol::schema::scalar::Incarnation;
use serde_json::json;

use super::{Ended, Launched, ProcessDirectory, ProcessInfoRead};
use crate::dispatch::lanes::agent_status::{ResolvedTerminal, TerminalDirectory};

#[derive(Debug, Default)]
struct Table(Mutex<BTreeMap<u64, NativeProcess>>);

impl ProcessTable for Table {
    fn read(&self, pid: ProcessPid) -> Result<NativeProcess, Unavailable> {
        let records = self.0.lock().map_err(|_| Unavailable::Raced)?;
        records
            .get(&pid.get())
            .cloned()
            .ok_or(Unavailable::ProcessUnavailable)
    }

    fn candidates(&self, _: u64) -> Result<Vec<ProcessPid>, Unavailable> {
        let records = self.0.lock().map_err(|_| Unavailable::Raced)?;
        Ok(records
            .keys()
            .filter_map(|pid| ProcessPid::try_from(*pid).ok())
            .collect())
    }

    fn argv(&self, _: ProcessPid) -> Field<Vec<String>> {
        Field::Unsupported(Unavailable::Platform)
    }

    fn cwd(&self, _: ProcessPid) -> Field<PathBuf> {
        Field::Unsupported(Unavailable::Platform)
    }
}

/// One terminal named `proc`, running incarnation 1 as pid 40 with token `t-1`.
#[derive(Debug, Default)]
struct Directory {
    ended: Mutex<Vec<Ended>>,
}

fn terminal() -> TerminalId {
    TerminalId::try_from(9).expect("terminal id")
}

fn owned(token: &str) -> ProcessHandle {
    ProcessHandle::new(
        ProcessPid::try_from(40).expect("pid"),
        ProcessStartToken::new(token).expect("token"),
    )
}

impl TerminalDirectory for Directory {
    fn resolve(&self, target: &Target) -> Result<ResolvedTerminal, TargetResolutionError> {
        match target {
            Target::Name(name) if name.as_str() == "proc" => Ok(ResolvedTerminal::new(
                MachineId::new(uuid::Uuid::from_u128(1)).expect("machine"),
                terminal(),
                TerminalGeneration::new(1).expect("generation"),
            )),
            _ => Err(TargetResolutionError::NotFound),
        }
    }
}

impl ProcessDirectory for Directory {
    fn launched(&self, _: TerminalId) -> Result<Launched, TargetResolutionError> {
        let record: ProcessRecord = serde_json::from_value(json!({
            "incarnation": "1", "pid": 40, "start_token": "t-1",
            "argv": ["/bin/sh"], "cwd": null, "started_ms": "1"
        }))
        .map_err(|_| TargetResolutionError::NotFound)?;
        Ok(Launched::new(record, Some(owned("t-1"))))
    }

    fn ended(&self, ended: Ended) {
        self.ended.lock().expect("ended log").push(ended);
    }
}

fn lane(records: Vec<NativeProcess>) -> ProcessInfoRead<Arc<Directory>, Table> {
    let table = Table::default();
    for record in records {
        table
            .0
            .lock()
            .expect("table")
            .insert(record.process.pid().get(), record);
    }
    ProcessInfoRead::new(Arc::new(Directory::default()), Arc::new(table))
}

fn params(value: serde_json::Value) -> TerminalProcessInfoParams {
    serde_json::from_value(value).expect("params")
}

fn ended(lane: &ProcessInfoRead<Arc<Directory>, Table>) -> Vec<Ended> {
    lane.directory.ended.lock().expect("ended log").clone()
}

#[test]
fn a_reused_pid_is_process_replaced_and_revokes_the_old_incarnation() {
    let table = lane(vec![NativeProcess::new(
        owned("t-2"),
        true,
        Terminal::Unsupported,
    )]);

    let refused = table
        .answer(&params(json!({"target": {"name": "proc"}})))
        .expect_err("reused");

    assert_eq!(refused.kind(), "process_replaced");
    assert_eq!(
        ended(&table),
        [Ended {
            terminal: terminal(),
            incarnation: Incarnation::new(1),
            proof: Liveness::Replaced
        }]
    );
}

#[test]
fn an_unsupported_foreground_answers_without_revoking_or_reporting_exit() {
    let windows = lane(vec![NativeProcess::new(
        owned("t-1"),
        true,
        Terminal::Unsupported,
    )]);

    let result = windows
        .answer(&params(json!({"target": {"name": "proc"}})))
        .expect("roles");
    let wire = serde_json::to_value(&result).expect("encode");

    assert_eq!(ended(&windows), []);
    assert_eq!(wire.pointer("/roles/launcher/value/pid"), Some(&json!(40)));
    assert_eq!(
        wire.pointer("/roles/foreground/reason"),
        Some(&json!("platform"))
    );
    assert_eq!(wire.pointer("/foreground"), Some(&json!(null)));
    assert_eq!(result.supported_fields, ["launcher", "pid", "start_token"]);
}

#[test]
fn the_launcher_in_the_foreground_group_is_the_legacy_foreground_record() {
    let session = Terminal::Controlling(Session::new(40, 3, 40));
    let shell = lane(vec![NativeProcess::new(owned("t-1"), true, session)]);

    let result = shell
        .answer(&params(json!({"target": {"name": "proc"}})))
        .expect("roles");

    assert_eq!(result.foreground.as_ref(), Some(&result.launcher));
    assert_eq!(
        result.supported_fields,
        ["launcher", "foreground", "background", "pid", "start_token"]
    );
}

#[test]
fn stale_expectations_are_refused_before_any_process_is_read() {
    let empty = lane(Vec::new());
    let ask = |expect| {
        empty.answer(&params(
            json!({"target": {"name": "proc"}, "expect": expect}),
        ))
    };

    let replaced = ask(json!({"process_incarnation": "2"})).expect_err("old incarnation");
    let stale = ask(json!({"terminal_incarnation": "7"})).expect_err("old terminal");

    assert_eq!(replaced.kind(), "process_replaced");
    assert_eq!(stale.kind(), "stale_target");
    assert_eq!(ended(&empty), []);
}

#[test]
fn an_unknown_name_is_not_found_and_never_another_terminal() {
    let running = lane(vec![NativeProcess::new(
        owned("t-1"),
        true,
        Terminal::Unsupported,
    )]);

    let refused = running
        .answer(&params(json!({"target": {"name": "focused"}})))
        .expect_err("unknown");

    assert_eq!(refused.kind(), "not_found");
}
