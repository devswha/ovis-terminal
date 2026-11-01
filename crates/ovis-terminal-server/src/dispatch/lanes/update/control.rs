//! The update service's restart port over the restartable control of plan todo 48.

use std::sync::Arc;

use ovis_terminal_core::restart::{Control, RestartError};
use ovis_terminal_core::terminals::TerminalError;
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::server::ServerTransitionResult;
use ovis_terminal_protocol::schema::scalar::Incarnation;
use ovis_terminal_services::update::{RestartControl, RestartFailure};

/// Restarts `Control` onto the selected build; holders keep every program.
#[derive(Debug)]
pub struct ControlRestart(Arc<Control>);

impl ControlRestart {
    /// Delegates restarts to `control`.
    #[must_use]
    pub const fn new(control: Arc<Control>) -> Self {
        Self(control)
    }
}

fn failure(error: RestartError) -> RestartFailure {
    RestartFailure::Control {
        kind: error.kind(),
        hint: error.hint(),
        source: Box::new(error),
    }
}

impl RestartControl for ControlRestart {
    fn restart(&self, op_id: &OpId) -> Result<ServerTransitionResult, RestartFailure> {
        let receipt = self.0.restart(op_id.clone()).map_err(failure)?;
        Ok(receipt.transition())
    }

    fn live_incarnations(&self) -> Result<Vec<Incarnation>, RestartFailure> {
        self.0
            .with_terminals(|terminals, session| {
                let records = terminals.list(session)?;
                Ok::<_, TerminalError>(
                    records
                        .iter()
                        .filter(|record| record.exit.is_none())
                        .filter_map(|record| record.lifecycle.process())
                        .map(|process| Incarnation::new(process.generation().get()))
                        .collect(),
                )
            })
            .map_err(failure)
    }
}
