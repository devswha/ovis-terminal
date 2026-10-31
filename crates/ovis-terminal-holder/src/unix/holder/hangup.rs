//! Explicit SIGHUP delivery through the holder's process-ownership boundary.

use super::Holder;
use super::signalling::{SignalOutcome, signal_with};
use crate::process::{SignalScope, send_hangup};
use crate::protocol::TerminalTarget;
use crate::unix::HolderError;

impl Holder {
    /// Sends SIGHUP to the exact held process group, without claiming process exit.
    ///
    /// Delivery is serialized with the waiter's reap and checked against the
    /// retained incarnation evidence, like every other signal: the group of a
    /// reaped leader is still addressed, a reused PID never is.
    /// This local owner API does not change the generation-two Signal wire enum.
    ///
    /// # Errors
    ///
    /// Returns stale identity, ended ownership or native signal delivery failures.
    pub fn hangup(&self, target: &TerminalTarget) -> Result<(), HolderError> {
        if target.process_incarnation.is_none() {
            return Err(HolderError::StaleTarget {
                terminal_id: target.terminal_id.clone(),
            });
        }
        self.shared.with_state(|state| {
            let terminal = state.terminal_mut(target)?;
            let live = terminal.live.as_ref().ok_or(HolderError::ProcessExited)?;
            let group = SignalScope::ProcessGroup(live.pid);
            match signal_with(terminal, group, send_hangup)? {
                SignalOutcome::Delivered { .. } => Ok(()),
                SignalOutcome::AlreadyExited => Err(HolderError::ProcessExited),
            }
        })
    }
}
