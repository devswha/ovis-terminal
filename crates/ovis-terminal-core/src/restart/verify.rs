//! What the successor proves about each incarnation the restart began with.

use ovis_terminal_holder::process::{ProcessId, StartTokenError, read_start_token};
use ovis_terminal_protocol::schema::scalar::EventSeq;

use super::control::Inner;
use super::receipt::{LossEvidence, Lost};
use super::{Preserved, RestartError, RestartIntent, RestartReceipt, classify};
use crate::registry::ProcessIdentity;
use crate::terminals::{Record, TerminalError};

impl Inner {
    pub(super) fn recover(&self, intent: &RestartIntent) -> Result<RestartReceipt, RestartError> {
        let terminals = self
            .terminals
            .as_ref()
            .ok_or(RestartError::ControlStopped)?;
        let session = self.plan.session();
        let records = terminals.list(session)?;
        let mut receipt = RestartReceipt {
            runtime: intent.runtime,
            from: intent.from,
            to: intent.to,
            preserved: Vec::new(),
            lost: Vec::new(),
            interrupted_ops: Vec::new(),
            interrupted_waits: intent.waits.clone(),
            seq_before: intent.seq,
            seq_after: intent.seq,
            resumed: false,
        };
        for held in &intent.preserved {
            let record = records.iter().find(|record| record.id() == held.terminal);
            match verdict(record, held)? {
                None => receipt.preserved.push(held.clone()),
                Some(evidence) => receipt.lost.push(Lost::new(held.clone(), evidence)),
            }
        }
        for op in &intent.in_flight {
            receipt
                .interrupted_ops
                .push(classify(&terminals.operation(session, op)?));
        }
        receipt.seq_after =
            EventSeq::new(terminals.coordinator().read_snapshot()?.cursor.seq.get());
        Ok(receipt)
    }
}

/// `None` when the successor adopted `held` and the operating system still runs it with
/// its recorded start token; otherwise the evidence of why it is not preserved.
fn verdict(
    record: Option<&Record>,
    held: &Preserved,
) -> Result<Option<LossEvidence>, RestartError> {
    let Some(record) = record else {
        return Ok(Some(LossEvidence::NotAdopted {}));
    };
    Ok(match record.lifecycle.process() {
        Some(process) if process == &held.process => {
            (!alive(process)?).then_some(LossEvidence::Gone {})
        }
        Some(found) => Some(LossEvidence::Replaced {
            found: found.clone(),
        }),
        None if record.lifecycle.last_process() == Some(&held.process) => {
            Some(record.exit.as_ref().map_or(
                LossEvidence::Gone {},
                |exit| match exit.exit_unknown {
                    None => LossEvidence::Exited {
                        code: exit.code,
                        signal: exit.signal,
                    },
                    Some(_) => LossEvidence::ExitUnknown {},
                },
            ))
        }
        None => Some(LossEvidence::Replaced {
            found: record
                .lifecycle
                .last_process()
                .cloned()
                .ok_or(TerminalError::ProcessReplaced)?,
        }),
    })
}

/// Whether the operating system still runs `process` with its recorded start token.
fn alive(process: &ProcessIdentity) -> Result<bool, RestartError> {
    let pid = u32::try_from(process.pid().get()).map_err(|_| TerminalError::ProcessReplaced)?;
    match read_start_token(ProcessId::new(pid)) {
        Ok(token) => Ok(token.as_str() == process.start_token().as_str()),
        Err(StartTokenError::ProcessGone { .. }) => Ok(false),
        Err(error) => Err(TerminalError::from(error).into()),
    }
}
