//! Explicit SIGHUP delivery through the holder's process-ownership boundary.

use rustix::process::{Pid, Signal, kill_process_group};

use super::Holder;
use crate::process::SignalError;
use crate::protocol::TerminalTarget;
use crate::unix::HolderError;

impl Holder {
    /// Sends SIGHUP to the exact held process group, without claiming process exit.
    ///
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
            let pid = i32::try_from(live.pid.get())
                .ok()
                .and_then(Pid::from_raw)
                .ok_or_else(|| SignalError::PidOutOfRange {
                    pid: live.pid.get(),
                })?;
            kill_process_group(pid, Signal::HUP).map_err(|source| SignalError::Delivery {
                source: source.into(),
            })?;
            Ok(())
        })
    }
}
