//! The terminals of one load run: one actor and one emitting PTY each, plus viewers.

use std::path::{Path, PathBuf};

use ovis_terminal_probes::platform::Pid;
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_server::streams::scheduler::{
    ActorTask, Engine, HistoryConfig, HistoryStore, OutputEvent, TerminalHandle, TerminalSetup,
    spawn_terminal,
};
use ovis_terminal_vt::{Geometry, HistoryLimit, Vt, VtConfig};
use tokio::sync::broadcast::Receiver;
use tokio::task::JoinSet;

use super::clients;
use super::error::TuningError;
use super::load::Load;
use super::pty::{Emit, EmitTerminal, Wiring};

pub(super) struct Fleet {
    pub(super) handles: Vec<TerminalHandle>,
    pub(super) tasks: Vec<ActorTask>,
    pub(super) ptys: Vec<EmitTerminal>,
    pub(super) pids: Vec<Pid>,
    pub(super) viewers: JoinSet<(u64, u64)>,
    pub(super) stalled: Vec<Receiver<OutputEvent>>,
    pub(super) history_root: PathBuf,
}

/// One actor with an advertised disk history and one emitting PTY per terminal; one
/// viewer per terminal never reads.
pub(super) fn spawn_fleet(
    exe: &Path,
    (load, emit, setup): (Load, Emit, &TerminalSetup),
    runtime: &tokio::runtime::Handle,
) -> Result<Fleet, TuningError> {
    let geometry = Geometry::new(120, 40)?;
    let history = HistoryConfig::advertised(&Limits::v1());
    let history_root = std::env::temp_dir().join(format!("ovis-tuning-{}", std::process::id()));
    let mut fleet = Fleet {
        handles: Vec::new(),
        tasks: Vec::new(),
        ptys: Vec::new(),
        pids: Vec::new(),
        viewers: JoinSet::new(),
        stalled: Vec::new(),
        history_root,
    };
    for terminal in 0..load.terminals {
        let (sink, stdin) = tokio::io::duplex(64 * 1024);
        let vt = Vt::new(VtConfig::new(geometry, HistoryLimit::new(2_000)));
        let store = HistoryStore::open(&fleet.history_root.join(terminal.to_string()), history)?;
        let engine = Engine::new(vt).with_history(store);
        let (handle, feed, task) = spawn_terminal(engine, sink, setup)?;
        for viewer in 0..load.viewers {
            if viewer == 0 {
                fleet.stalled.push(handle.subscribe_output());
            } else {
                fleet
                    .viewers
                    .spawn(clients::view(handle.subscribe_output()));
            }
        }
        let wiring = Wiring {
            feed,
            stdin,
            runtime: runtime.clone(),
        };
        let pty = EmitTerminal::spawn(exe, emit, wiring)?;
        fleet.pids.push(pty.pid());
        fleet.ptys.push(pty);
        fleet.handles.push(handle);
        fleet.tasks.push(task);
    }
    Ok(fleet)
}
