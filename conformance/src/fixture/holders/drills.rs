//! The four holder drills, each against real holders over their sockets.

use std::path::Path;

use ovis_terminal_core::recovery::{
    Adopted, GroupId, OsBootId, Recorded, RecoveryPlan, read_os_boot, recover,
};
use ovis_terminal_holder::process::{ProcessId, read_start_token};
use ovis_terminal_holder::protocol::{Message, TerminalId, TerminalTarget};
use ovis_terminal_holder::unix::HolderLimits;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::ids::{RuntimeLifetimeId, ServerBootId};
use ovis_terminal_vt::{Geometry, HistoryLimit, Vt, VtConfig};
use serde_json::{Value, json};

use super::super::machines::internal;
use super::{Drill, Served};

/// Writes 1240 bytes and `MARK`, waits until the `go` FIFO is written, then writes about
/// 11 KiB more and `DONE` and waits for input; the checkpoint is therefore taken exactly at
/// `MARK`, whatever the scheduling.
const WRITER: &str = "i=0; while [ $i -lt 40 ]; do printf '%030d\\r' $i; i=$((i+1)); done; printf MARK; read go < go; while [ $i -lt 400 ]; do printf '%030d\\r' $i; i=$((i+1)); done; printf DONE; read gate";

/// Retention small enough that the crash drill's output after `MARK` overflows it.
const SMALL_TAIL: usize = 4096;

fn plan(state: &Path) -> Result<RecoveryPlan, BoundaryError> {
    let geometry = Geometry::new(80, 24).map_err(internal)?;
    let identity = (
        RuntimeLifetimeId::new("runtime:52".to_owned()),
        ServerBootId::new("server-boot:2".to_owned()),
    );
    let config = VtConfig::new(geometry, HistoryLimit::new(2000));
    Ok(RecoveryPlan::new(state.to_path_buf(), identity, config))
}

fn adopt(served: &Served, plan: &RecoveryPlan) -> Result<Adopted, BoundaryError> {
    Adopted::adopt(&served.group, plan.core_build.clone(), plan.bound).map_err(internal)
}

/// Every output byte of `target` read from `link` until `marker` was seen.
fn until(
    link: &mut Adopted,
    (target, marker): (&TerminalTarget, &[u8]),
) -> Result<Vec<u8>, BoundaryError> {
    let mut seen = Vec::new();
    while !seen.windows(marker.len()).any(|bytes| bytes == marker) {
        if let Message::Output(output) = link.receive().map_err(internal)?
            && output.target.terminal_id == target.terminal_id
        {
            seen.extend_from_slice(&output.bytes);
        }
    }
    Ok(seen)
}

/// Whether the operating system still runs `recorded` under its start token.
fn running(recorded: &Recorded) -> bool {
    recorded.pid > 1
        && read_start_token(ProcessId::new(recorded.pid)).is_ok_and(|t| t == recorded.start_token)
}

/// Runs `/bin/sh -c "printf 'first '; printf LAST-<placement>"` and reads it to its exit.
fn to_exit(
    served: &Served,
    (plan, placement): (&RecoveryPlan, &str),
) -> Result<Value, BoundaryError> {
    let mut link = adopt(served, plan)?;
    let last = format!("LAST-{placement}");
    let script = format!("printf 'first '; printf {last}");
    let recorded = served.spawn(&format!("terminal:{placement}"), &script)?;
    let mut output = Vec::new();
    let exit = loop {
        match link.receive().map_err(internal)? {
            Message::Output(chunk) if chunk.target.terminal_id == recorded.target.terminal_id => {
                output.extend_from_slice(&chunk.bytes);
            }
            Message::Exit(exit) if exit.target.terminal_id == recorded.target.terminal_id => {
                break exit;
            }
            _ => {}
        }
    };
    Ok(json!({
        "placement": placement,
        "output": String::from_utf8_lossy(&output),
        "final_before_exit": output.ends_with(last.as_bytes()),
        "bytes_before_exit": output.len().to_string(),
        "final_offset": exit.final_offset.to_string(),
        "exit_code": exit.code,
    }))
}

fn exit(state: &Path) -> Result<Value, BoundaryError> {
    let plan = plan(state)?;
    let shared = Served::new(
        state,
        (
            GroupId::shared("fixture", "task-52"),
            HolderLimits::DEFAULT.tail_bytes,
        ),
    )?;
    let terminal = TerminalId::new("terminal:dedicated".to_owned());
    let dedicated = Served::new(
        state,
        (
            GroupId::dedicated(&terminal),
            HolderLimits::DEFAULT.tail_bytes,
        ),
    )?;
    let report = json!({
        "shared": to_exit(&shared, (&plan, "shared"))?,
        "dedicated": to_exit(&dedicated, (&plan, "dedicated"))?,
    });
    shared.close()?;
    dedicated.close()?;
    Ok(report)
}

fn crash(state: &Path) -> Result<Value, BoundaryError> {
    let plan = plan(state)?;
    let served = Served::new(state, (GroupId::shared("fixture", "task-52"), SMALL_TAIL))?;
    let fifo = state.join("go");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .map_err(internal)?;
    if !made.success() {
        return Err(internal("mkfifo failed"));
    }
    let mut old = adopt(&served, &plan)?;
    let recorded = served.spawn("terminal:writer", WRITER)?;
    let seen = until(&mut old, (&recorded.target, b"MARK"))?;
    let mut crashed = Vt::new(plan.config);
    crashed.feed(&seen);
    let checkpoint = crashed.export_checkpoint();
    plan.checkpoints()
        .save(&recorded.target, &checkpoint)
        .map_err(internal)?;
    plan.stamp()
        .record(&read_os_boot().map_err(internal)?)
        .map_err(internal)?;
    let mut go = std::fs::OpenOptions::new()
        .write(true)
        .open(&fifo)
        .map_err(internal)?;
    std::io::Write::write_all(&mut go, b"go\n").map_err(internal)?;
    drop(go);
    until(&mut old, (&recorded.target, b"DONE"))?;
    drop(old);
    let recovered = recover(
        &plan,
        std::slice::from_ref(&served.group),
        std::slice::from_ref(&recorded),
    )
    .map_err(internal)?;
    let report = json!({
        "report": recovered.report,
        "checkpoint_offset": checkpoint.offset().get().to_string(),
        "running": running(&recorded),
    });
    drop(recovered);
    served.close()?;
    Ok(report)
}

fn reboot(state: &Path) -> Result<Value, BoundaryError> {
    let plan = plan(state)?;
    let served = Served::new(
        state,
        (
            GroupId::shared("fixture", "task-52"),
            HolderLimits::DEFAULT.tail_bytes,
        ),
    )?;
    let mut old = adopt(&served, &plan)?;
    let recorded = served.spawn("terminal:idle", "printf ARMED; read gate")?;
    until(&mut old, (&recorded.target, b"ARMED"))?;
    drop(old);
    plan.stamp()
        .record(&OsBootId::new("linux:a-previous-boot".to_owned()))
        .map_err(internal)?;
    let recovered = recover(
        &plan,
        std::slice::from_ref(&served.group),
        std::slice::from_ref(&recorded),
    )
    .map_err(internal)?;
    let report = json!({
        "report": recovered.report,
        "adopted": recovered.links.len().to_string(),
        "running": running(&recorded),
    });
    drop(recovered);
    served.close()?;
    Ok(report)
}

fn unplug(state: &Path) -> Result<Value, BoundaryError> {
    let plan = plan(state)?;
    let shared = Served::new(
        state,
        (
            GroupId::shared("fixture", "task-52"),
            HolderLimits::DEFAULT.tail_bytes,
        ),
    )?;
    let terminal = TerminalId::new("terminal:alone".to_owned());
    let dedicated = Served::new(
        state,
        (
            GroupId::dedicated(&terminal),
            HolderLimits::DEFAULT.tail_bytes,
        ),
    )?;
    let mut links = (adopt(&shared, &plan)?, adopt(&dedicated, &plan)?);
    let kept = shared.spawn("terminal:kept", "printf ARMED; read gate")?;
    let alone = dedicated.spawn("terminal:alone", "printf ARMED; read gate")?;
    until(&mut links.0, (&kept.target, b"ARMED"))?;
    until(&mut links.1, (&alone.target, b"ARMED"))?;
    plan.stamp()
        .record(&read_os_boot().map_err(internal)?)
        .map_err(internal)?;
    drop(links);
    dedicated.unplug()?;
    let groups = [shared.group.clone(), dedicated.group.clone()];
    let recovered = recover(&plan, &groups, &[kept.clone(), alone.clone()]).map_err(internal)?;
    let report = json!({
        "report": recovered.report,
        "running": [running(&kept), running(&alone)],
    });
    drop(recovered);
    shared.close()?;
    dedicated.close()?;
    Ok(report)
}

/// Runs `drill` in `state`.
pub(super) fn run(state: &Path, drill: Drill) -> Result<Value, BoundaryError> {
    match drill {
        Drill::Exit => exit(state),
        Drill::Crash => crash(state),
        Drill::Reboot => reboot(state),
        Drill::Unplug => unplug(state),
    }
}
