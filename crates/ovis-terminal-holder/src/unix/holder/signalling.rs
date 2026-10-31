//! Incarnation-checked delivery, serialized with the waiter's actual reap.

use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};

use super::state::{Terminal, WaiterState};
use crate::process::{SignalError, SignalScope, StartTokenError, read_start_token, send_signal};
use crate::protocol::SignalKind;
use crate::unix::error::HolderError;

#[derive(Debug, Eq, PartialEq)]
pub(super) enum SignalOutcome {
    Delivered { leader_exited: bool },
    AlreadyExited,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LeaderState {
    Live,
    Exited,
    Replaced,
}

/// The caller holds the holder state mutex until delivery/reclassification ends.
pub(super) fn signal(
    terminal: &Terminal,
    scope: SignalScope,
    kind: SignalKind,
) -> Result<SignalOutcome, HolderError> {
    let leader = leader_state(terminal)?;
    match (scope, leader) {
        (SignalScope::Process(_), LeaderState::Exited | LeaderState::Replaced)
        | (SignalScope::ProcessGroup(_), LeaderState::Replaced) => {
            return Ok(SignalOutcome::AlreadyExited);
        }
        (SignalScope::Process(_) | SignalScope::ProcessGroup(_), LeaderState::Live)
        | (SignalScope::ProcessGroup(_), LeaderState::Exited) => {}
    }
    match send_signal(scope, kind) {
        Ok(()) => Ok(SignalOutcome::Delivered {
            leader_exited: leader == LeaderState::Exited,
        }),
        Err(error) => failed_delivery(terminal, error),
    }
}

fn leader_state(terminal: &Terminal) -> Result<LeaderState, HolderError> {
    let Some(live) = &terminal.live else {
        return Ok(LeaderState::Exited);
    };
    match read_start_token(live.pid) {
        Ok(_) if matches!(terminal.waiter, WaiterState::Reaped) => {
            return Ok(LeaderState::Replaced);
        }
        Ok(actual) if terminal.held.start_token.as_ref() == Some(&actual) => {}
        Ok(_) => return Ok(LeaderState::Replaced),
        Err(StartTokenError::ProcessGone { .. }) => return Ok(LeaderState::Exited),
        Err(source) => return Err(HolderError::ProcessStart { source }),
    }
    let invalid_pid = || SignalError::PidOutOfRange {
        pid: live.pid.get(),
    };
    let raw = i32::try_from(live.pid.get()).map_err(|_| invalid_pid())?;
    let pid = Pid::from_raw(raw).ok_or_else(invalid_pid)?;
    waitid(
        WaitId::Pid(pid),
        WaitIdOptions::EXITED | WaitIdOptions::NOWAIT | WaitIdOptions::NOHANG,
    )
    .map(|status| {
        if status.is_some() {
            LeaderState::Exited
        } else {
            LeaderState::Live
        }
    })
    .map_err(|source| HolderError::ProcessWait {
        source: source.into(),
    })
}

/// Only fresh exit/incarnation evidence can turn a delivery failure into exit.
fn failed_delivery(terminal: &Terminal, error: SignalError) -> Result<SignalOutcome, HolderError> {
    match &error {
        SignalError::Delivery { source } => match source.raw_os_error() {
            Some(libc::ESRCH) => return Ok(SignalOutcome::AlreadyExited),
            Some(libc::EPERM) => {
                if leader_state(terminal)? != LeaderState::Live {
                    return Ok(SignalOutcome::AlreadyExited);
                }
            }
            Some(_) | None => {}
        },
        SignalError::PidOutOfRange { .. } => {}
    }
    Err(error.into())
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::{LeaderState, SignalOutcome, failed_delivery, leader_state, signal};
    use crate::process::{SignalError, SignalScope};
    use crate::protocol::{ProcessStartToken, SignalKind, Spawned};
    use crate::unix::Holder;
    use crate::unix::error::HolderError;
    use crate::unix::test_fixtures::{config, shell};

    fn live_holder() -> (Holder, Spawned) {
        let holder = Holder::new(config());
        let spawned = holder
            .spawn_wire(&shell("t-live", "exec sleep 600"))
            .expect("real live child");
        (holder, spawned)
    }

    #[test]
    fn mismatched_start_token_never_signals_a_live_pid() {
        // Given: a real live PID whose retained incarnation evidence is foreign.
        let (holder, spawned) = live_holder();
        let (outcome, survived) = {
            let mut state = holder.shared.lock();
            let terminal = state.terminals.get_mut(&spawned.terminal_id).expect("held");
            let original = terminal.held.start_token.replace(ProcessStartToken::new(
                "different-process-incarnation".to_owned(),
            ));
            let scope = SignalScope::ProcessGroup(terminal.live.as_ref().expect("live").pid);

            // When: the stale incarnation requests an actual kill.
            let outcome = signal(terminal, scope, SignalKind::Kill);
            terminal.held.start_token = original;
            let survived =
                leader_state(terminal).expect("real exit readiness") == LeaderState::Live;
            drop(state);
            (outcome, survived)
        };
        holder.shutdown().expect("stop and join owned child");

        // Then: the different incarnation is not touched.
        assert_eq!(
            outcome.expect("stale outcome"),
            SignalOutcome::AlreadyExited
        );
        assert!(survived);
    }

    #[test]
    fn permission_denied_is_preserved_while_target_is_live() {
        // Given: matching incarnation evidence for a real live child.
        let (holder, spawned) = live_holder();
        let outcome = {
            let state = holder.shared.lock();
            let terminal = state.terminals.get(&spawned.terminal_id).expect("held");

            // When: delivery's typed EPERM reaches the reclassification boundary.
            let outcome = failed_delivery(
                terminal,
                SignalError::Delivery {
                    source: io::Error::from_raw_os_error(libc::EPERM),
                },
            );
            drop(state);
            outcome
        };
        holder.shutdown().expect("stop and join owned child");

        // Then: live-target EPERM remains a sourced, typed delivery error.
        let error = outcome.expect_err("live EPERM must not become success");
        assert_eq!(error.kind(), "signal_failed");
        assert!(matches!(
            error,
            HolderError::Signal {
                source: SignalError::Delivery { source }
            } if source.raw_os_error() == Some(libc::EPERM)
        ));
    }
}
