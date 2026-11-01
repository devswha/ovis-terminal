//! The terminals of one run: an actor with a disk history and one program each, sharing
//! one set of session budgets, plus output viewers of which the first never reads.

use std::path::Path;
use std::thread::JoinHandle;

use ovis_terminal_probes::platform::Pid;
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_server::streams::scheduler::{
    ActorConfig, ActorTask, Engine, HistoryConfig, HistoryStore, OutputEvent, OutputFeed,
    TerminalHandle, TerminalSetup, spawn_terminal,
};
use ovis_terminal_vt::{Geometry, HistoryLimit, Vt, VtConfig};
use tokio::io::DuplexStream;
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::oneshot;
use tokio::task::JoinSet;

use super::error::Terminal50Error;
use super::plan::{Plan, Source};
use super::pty::{Emit, EmitTerminal};

/// The actor-side ends one program is wired to.
#[derive(Debug)]
pub(super) struct Wiring {
    pub(super) feed: OutputFeed,
    pub(super) stdin: DuplexStream,
    pub(super) runtime: tokio::runtime::Handle,
}

/// What feeds one terminal.
#[derive(Debug)]
pub(super) enum Program {
    Pty(EmitTerminal),
    #[cfg(test)]
    Feed {
        task: tokio::task::JoinHandle<()>,
        stop: tokio::sync::watch::Sender<bool>,
        unread_stdin: DuplexStream,
    },
}

/// What one reading viewer received.
#[derive(Debug, Clone, Copy)]
pub(super) struct Viewed {
    pub(super) terminal: usize,
    pub(super) viewer: usize,
    pub(super) bytes: u64,
    pub(super) lost: u64,
}

#[derive(Debug)]
pub(super) struct Fleet {
    pub(super) handles: Vec<TerminalHandle>,
    pub(super) tasks: Vec<ActorTask>,
    pub(super) programs: Vec<Program>,
    /// Pids of the PTY programs, captured at spawn; the only processes ever signalled.
    pub(super) pids: Vec<Pid>,
    pub(super) viewers: JoinSet<Viewed>,
    pub(super) stalled: Vec<(usize, Receiver<OutputEvent>)>,
    pub(super) ready: Vec<oneshot::Receiver<()>>,
}

async fn view(
    (terminal, viewer): (usize, usize),
    mut output: Receiver<OutputEvent>,
    ready: Option<oneshot::Sender<()>>,
) -> Viewed {
    let mut seen = Viewed {
        terminal,
        viewer,
        bytes: 0,
        lost: 0,
    };
    let mut ready = ready;
    loop {
        match output.recv().await {
            Ok(OutputEvent::Chunk { bytes, .. }) => {
                let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                seen.bytes = seen.bytes.saturating_add(len);
                if let Some(signal) = ready.take() {
                    match signal.send(()) {
                        Ok(()) | Err(()) => {}
                    }
                }
            }
            Ok(_) => {}
            Err(RecvError::Lagged(missed)) => seen.lost = seen.lost.saturating_add(missed),
            Err(RecvError::Closed) => return seen,
        }
    }
}

#[cfg(test)]
async fn feed(
    feed: OutputFeed,
    bytes_per_second: usize,
    mut stop: tokio::sync::watch::Receiver<bool>,
) {
    let slice = super::emit::corpus(bytes_per_second.div_ceil(100).max(1), 0);
    let mut pace = tokio::time::interval(std::time::Duration::from_millis(10));
    loop {
        tokio::select! {
            _ = stop.changed() => return,
            _ = pace.tick() => {
                if feed.send(&slice).await.is_err() {
                    return;
                }
            }
        }
    }
}

fn program(plan: &Plan, wiring: Wiring, pids: &mut Vec<Pid>) -> Result<Program, Terminal50Error> {
    let bytes_per_second = plan.per_terminal_rate();
    match &plan.source {
        Source::Pty { exe } => {
            let emit = Emit {
                exe,
                root: &plan.root,
                bytes_per_second,
            };
            let pty = EmitTerminal::spawn(emit, wiring)?;
            pids.push(pty.pid());
            Ok(Program::Pty(pty))
        }
        #[cfg(test)]
        Source::InProcess => {
            let (stop, stopped) = tokio::sync::watch::channel(false);
            let task = wiring
                .runtime
                .spawn(feed(wiring.feed, bytes_per_second, stopped));
            Ok(Program::Feed {
                task,
                stop,
                unread_stdin: wiring.stdin,
            })
        }
    }
}

/// Spawns every terminal, program and viewer; subscriptions exist before any load.
pub(super) fn spawn(
    plan: &Plan,
    history_root: &Path,
    runtime: &tokio::runtime::Handle,
) -> Result<Fleet, Terminal50Error> {
    let limits = Limits::v1();
    let setup = TerminalSetup::advertised(ActorConfig::advertised(&limits), &limits);
    let history = HistoryConfig::advertised(&limits);
    let geometry = Geometry::new(120, 40)?;
    let mut fleet = Fleet {
        handles: Vec::new(),
        tasks: Vec::new(),
        programs: Vec::new(),
        pids: Vec::new(),
        viewers: JoinSet::new(),
        stalled: Vec::new(),
        ready: Vec::new(),
    };
    for terminal in 0..plan.shape.terminals {
        let (sink, stdin) = tokio::io::duplex(64 * 1024);
        let vt = Vt::new(VtConfig::new(geometry, HistoryLimit::new(2_000)));
        let store = HistoryStore::open(&history_root.join(terminal.to_string()), history)?;
        let (handle, feed, task) =
            spawn_terminal(Engine::new(vt).with_history(store), sink, &setup)?;
        for viewer in 0..plan.viewers {
            let output = handle.subscribe_output();
            if viewer == 0 {
                fleet.stalled.push((terminal, output));
                continue;
            }
            let signal = (viewer == 1).then(|| {
                let (signal, ready) = oneshot::channel();
                fleet.ready.push(ready);
                signal
            });
            fleet
                .viewers
                .spawn_on(view((terminal, viewer), output, signal), runtime);
        }
        let wiring = Wiring {
            feed,
            stdin,
            runtime: runtime.clone(),
        };
        let program = program(plan, wiring, &mut fleet.pids)?;
        fleet.programs.push(program);
        fleet.handles.push(handle);
        fleet.tasks.push(task);
    }
    Ok(fleet)
}

/// Stops one program; a PTY's stdin forwarder is returned to join after its actor stops.
pub(super) async fn stop(
    program: Program,
) -> Result<(u64, Option<JoinHandle<u64>>), Terminal50Error> {
    match program {
        Program::Pty(pty) => {
            let (pumped, forward) = tokio::task::spawn_blocking(move || pty.finish())
                .await
                .map_err(std::io::Error::other)??;
            Ok((pumped.bytes, Some(forward)))
        }
        #[cfg(test)]
        Program::Feed {
            task,
            stop,
            unread_stdin,
        } => {
            stop.send_replace(true);
            task.await.map_err(std::io::Error::other)?;
            drop(unread_stdin);
            Ok((0, None))
        }
    }
}
