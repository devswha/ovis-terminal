//! Launching into a created pane; a failure is an outcome, never a lost creation.

use ovis_terminal_authz::Session;

use super::LaunchOutcome;
use crate::terminals::Launch;

/// Starts a program for a pane that a creating edit already committed.
pub trait Launcher: std::fmt::Debug {
    /// Launches `launch` on behalf of `session` and reports the separate outcome.
    fn launch(&mut self, session: &Session, launch: Launch) -> LaunchOutcome;
}

#[cfg(unix)]
impl Launcher for crate::terminals::Terminals {
    fn launch(&mut self, session: &Session, launch: Launch) -> LaunchOutcome {
        use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind, OpId};

        let op_id = OpId::new(uuid::Uuid::now_v7().to_string());
        match self.run(crate::terminals::Call::new(session, op_id.clone()), launch) {
            Ok(record) => LaunchOutcome::Launched {
                terminal: record.id(),
                spawn: record.spawn,
            },
            Err(error) => LaunchOutcome::Rejected {
                error: Box::new(
                    BoundaryError::domain(ErrorKind::from(error.kind().to_owned()), false)
                        .with_hint(error.hint())
                        .with_op_id(op_id)
                        .data()
                        .clone(),
                ),
            },
        }
    }
}
