//! `server.stop` over the restart control the server booted with, and the signal that
//! ends serving once a stop was committed.
//!
//! `server.restart` is not routed: a planned restart hands the lifecycle to a successor
//! boot identity, and the listener, lease log and dispatch log of this lifetime would keep
//! reporting the old one. Until the listener is replaced at the restart boundary the
//! method stays `method_not_found` rather than half served.

use std::sync::Arc;

use ovis_terminal_core::registry::MachineId;
use ovis_terminal_core::restart::Control;
use ovis_terminal_server::dispatch::lanes::restart::{RestartLane, ServerStop};
use tokio::sync::Notify;

use super::AssemblyError;
use super::lanes::Dispatch;

/// Registers `server.stop` for `machine` over `control`.
pub(super) fn register(
    dispatch: &mut Dispatch,
    control: Arc<Control>,
    machine: MachineId,
) -> Result<(), AssemblyError> {
    let lane = RestartLane::new(control, machine);
    dispatch
        .register(ServerStop::new(&lane))
        .map_err(AssemblyError::start("register server.stop"))
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
