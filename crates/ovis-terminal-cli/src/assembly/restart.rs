//! `server.stop` and `server.restart` over the restart control the server booted with, and
//! the signals the serve loop follows: a committed stop ends serving, and a call that may
//! have restarted control asks the loop to hand everything over to the successor boot.

use std::sync::{Arc, Mutex, PoisonError};

use ovis_terminal_core::registry::MachineId;
use ovis_terminal_core::restart::Control;
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

/// The operation of the latest call that may have restarted control. The serve loop
/// compares the control's identity with the one it serves, so a refused restart hands
/// nothing over.
#[derive(Debug, Clone, Default)]
pub(super) struct RestartSignal(Arc<(Notify, Mutex<Option<String>>)>);

impl RestartSignal {
    /// Records `op`, whose call may have restarted control.
    pub(super) fn requested(&self, op: String) {
        let (notify, pending) = &*self.0;
        *pending.lock().unwrap_or_else(PoisonError::into_inner) = Some(op);
        notify.notify_one();
    }

    /// Resolves with the operation of the next request.
    pub(super) async fn wait(&self) -> String {
        let (notify, pending) = &*self.0;
        loop {
            notify.notified().await;
            let op = pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(op) = op {
                return op;
            }
        }
    }
}
