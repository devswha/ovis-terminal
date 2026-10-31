//! Proven pre-spawn rejection is different from uncertain worker completion.

use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_store::{CasWrite, Payload, Version};

use super::super::Terminals;
use crate::events::ResourceId;
use crate::ops::{Completion, Operations};
use crate::registry::TerminalId;
use crate::terminals::snapshot::{read_record, record_key};
use crate::terminals::{LaunchState, LifecycleEvent, TerminalError};

impl Terminals {
    pub(in crate::terminals::runtime) fn rejected(
        &mut self,
        id: TerminalId,
        error: TerminalError,
    ) -> Result<(), TerminalError> {
        let mut record = self.record(id)?;
        let rejection = BoundaryError::domain(ErrorKind::from(error.kind().to_owned()), false)
            .with_hint(error.hint())
            .with_op_id(record.launch_op.clone())
            .data()
            .clone();
        record.rejection = Some(rejection.clone());
        if let Some(pending) = self.pending.remove(&id) {
            let operations = Operations::new(&self.core, pending.context);
            match &error {
                TerminalError::Authority(_) | TerminalError::Worker(_) => {
                    record.spawn = LaunchState::Failed;
                    operations.commit_unit(pending.permit, |snapshot| {
                        let (_, version) = read_record(snapshot, id)?;
                        super::super::stamp_operation(&mut record, snapshot)?;
                        Ok(Completion::failed(rejection)
                            .write(CasWrite::new(
                                record_key(id),
                                Version::new(version.get()),
                                Some(Payload::from(serde_json::to_vec(&record)?)),
                            ))
                            .event(
                                LifecycleEvent::Updated(record.clone()),
                                vec![ResourceId::Terminal(id)],
                            ))
                    })?;
                }
                _ => {
                    operations.outcome_unknown(pending.permit)?;
                    self.persist(&record, LifecycleEvent::Updated(record.clone()))?;
                }
            }
        }
        Err(error)
    }
}
