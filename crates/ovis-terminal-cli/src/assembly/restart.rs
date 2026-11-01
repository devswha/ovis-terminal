//! `server.stop` and `server.restart` over the restart control the server booted with, and
//! the signals the serve loop follows: a committed stop ends serving, and a call that may
//! have restarted control asks the loop to hand everything over to the successor boot.

use std::sync::{Arc, Mutex, PoisonError};

use ovis_terminal_core::registry::MachineId;
use ovis_terminal_core::restart::Control;
use ovis_terminal_protocol::domain::restart::{BoundaryPhase, RestartBoundary};
use ovis_terminal_server::dispatch::lanes::restart::{RestartLane, ServerRestart, ServerStop};
use tokio::sync::Notify;

use super::AssemblyError;
use super::lanes::Dispatch;

/// Registers `server.stop` and `server.restart` for `machine` over `control`.
pub(super) fn register(
    dispatch: &mut Dispatch,
    control: Arc<Control>,
    machine: MachineId,
) -> Result<(), AssemblyError> {
    let lane = RestartLane::new(control, machine);
    dispatch
        .register(ServerStop::new(&lane))
        .map_err(AssemblyError::start("register server.stop"))?;
    dispatch
        .register(ServerRestart::new(&lane))
        .map_err(AssemblyError::start("register server.restart"))
}

/// Ends serving after a committed `server.stop`.
#[derive(Debug, Clone, Default)]
pub(super) struct StopSignal(Arc<Notify>);

impl StopSignal {
    /// Marks the stop committed; a waiter that arrives later still sees it.
    pub(super) fn stopped(&self) {
        self.0.notify_one();
    }

    /// Resolves once a stop was committed.
    pub(super) async fn wait(&self) {
        self.0.notified().await;
    }
}

/// The operation of the latest call that may have restarted control, for the hand-over's
/// journal entries. It never decides whether a hand-over happens: a call can answer (with
/// its deadline's pending outcome) before control restarts, so the serve loop follows the
/// restart boundaries control announces instead.
#[derive(Debug, Clone)]
pub(super) struct RestartSignal(Arc<(Mutex<Option<String>>, tokio::sync::watch::Sender<usize>)>);

impl Default for RestartSignal {
    fn default() -> Self {
        Self(Arc::new((
            Mutex::new(None),
            tokio::sync::watch::Sender::new(0),
        )))
    }
}

impl RestartSignal {
    /// A call that may restart control has begun; the hand-over waits until it answered.
    pub(super) fn begun(&self) {
        self.0
            .1
            .send_modify(|calls| *calls = calls.saturating_add(1));
    }

    /// A call that may have restarted control under `op` has its answer.
    pub(super) fn answered(&self, op: Option<String>) {
        if let Some(op) = op {
            *self.0.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(op);
        }
        self.0
            .1
            .send_modify(|calls| *calls = calls.saturating_sub(1));
    }

    /// Resolves once no restarting call is still waiting for its answer, so its reply is
    /// written before the hand-over closes its connection. The count is remembered, so a
    /// call that answered first is never waited for.
    pub(super) async fn quiet(&self) {
        let mut calls = self.0.1.subscribe();
        // The sender lives in `self`, so the channel cannot close while this waits.
        match calls.wait_for(|calls| *calls == 0).await {
            Ok(_) | Err(_) => {}
        }
    }

    /// The recorded operation, if any, leaving none.
    pub(super) fn take(&self) -> Option<String> {
        self.0
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

/// Completed restarts, as control announces them, delivered to the serve loop.
pub(super) type Completions = tokio::sync::mpsc::UnboundedReceiver<RestartBoundary>;

/// Forwards every completed restart boundary of `control` to the returned receiver. The
/// queue keeps each announcement, so a restart that completes while the serve loop is busy
/// is handed over afterwards; the forwarder ends with control.
pub(super) fn completions(control: &Control) -> Result<Completions, AssemblyError> {
    let boundaries = control
        .boundaries(BOUNDARY_QUEUE)
        .map_err(AssemblyError::start("listen for restart boundaries"))?;
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    std::thread::Builder::new()
        .name("ovis-restarts".into())
        .spawn(move || {
            for boundary in boundaries {
                let moved = boundary.server_boot_id_new != boundary.server_boot_id_old;
                if boundary.phase == BoundaryPhase::Completed
                    && moved
                    && sender.send(boundary).is_err()
                {
                    return;
                }
            }
        })
        .map_err(AssemblyError::start("start the restart forwarder"))?;
    Ok(receiver)
}

/// Boundaries queued for the forwarder; it drains them at once, so a restart never waits.
const BOUNDARY_QUEUE: usize = 16;
