//! Join actual holder write and exit records without inferring process termination.

use ovis_terminal_holder::protocol::{Exit, Input, InputWritten, TerminalTarget};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::TerminalId;
use ovis_terminal_protocol::schema::methods::server::{
    AckEpoch, FenceEpoch, GrantEpoch, OpRecord, OperationEvidence, ProcessExitEvidence,
    WriteEvidence, WriteIntent, WriteSequence,
};
use ovis_terminal_protocol::schema::scalar::{ByteCount, ByteOffset, Incarnation};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{OperationError, OperationEvent, OperationSnapshot, Operations, RunningOperation};
use crate::events::{EventData, SnapshotProducer};

fn target_evidence(target: &TerminalTarget) -> OperationEvidence {
    OperationEvidence::for_process(
        TerminalId::new(target.terminal_id.as_str().to_owned()),
        (
            Incarnation::new(target.terminal_incarnation.get()),
            target
                .process_incarnation
                .map(|value| Incarnation::new(value.get())),
        ),
    )
}

fn matches_target(evidence: &OperationEvidence, target: &TerminalTarget) -> bool {
    let actual = target_evidence(target);
    evidence.terminal_id == actual.terminal_id && evidence.incarnations == actual.incarnations
}

impl<D, P> Operations<'_, D, P>
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned,
    P: SnapshotProducer,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    /// Persists the exact holder input identity before the caller submits it.
    ///
    /// # Errors
    ///
    /// Returns stale permission, target mismatch or durable commit failures.
    pub fn bind_input(
        &self,
        permit: &RunningOperation,
        input: &Input,
    ) -> Result<OpRecord, OperationError> {
        let mut row = self.permitted(permit)?;
        if input.target.process_incarnation != Some(input.grant.process_incarnation) {
            return Err(OperationError::EvidenceMismatch);
        }
        let mut evidence = target_evidence(&input.target);
        evidence.intent = Some(
            WriteIntent::new(
                WriteSequence::new(input.sequence),
                GrantEpoch::new(input.grant.grant_epoch),
                FenceEpoch::new(input.grant.fence_epoch),
            )
            .with_bytes(ByteCount::new(
                u64::try_from(input.bytes.len()).map_err(|_| OperationError::TooLarge)?,
            )),
        );
        match &row.entry.record.evidence {
            Some(previous) if previous != &evidence => {
                return Err(OperationError::EvidenceMismatch);
            }
            Some(_) => return Ok(row.entry.record),
            None => row.entry.record.evidence = Some(evidence),
        }
        self.update(row)
    }

    /// Adds actual holder acknowledgement facts, not a success inferred from status.
    ///
    /// # Errors
    ///
    /// Returns `stale_target` for a foreign target, input entry, authority or acknowledgement.
    pub fn holder_write(&self, op: &OpId, ack: &InputWritten) -> Result<OpRecord, OperationError> {
        let mut row = self.current(op)?;
        let evidence = row
            .entry
            .record
            .evidence
            .as_mut()
            .ok_or(OperationError::EvidenceMismatch)?;
        let intent = evidence
            .intent
            .clone()
            .ok_or(OperationError::EvidenceMismatch)?;
        if !matches_target(evidence, &ack.target)
            || intent.sequence.get() != ack.sequence
            || intent.grant_epoch.get() != ack.grant_epoch
            || intent.fence_epoch.get() != ack.fence_epoch
            || ack.written_bytes > intent.requested_bytes.get()
        {
            return Err(OperationError::EvidenceMismatch);
        }
        let write = WriteEvidence::new(
            (
                ByteCount::new(ack.written_bytes),
                ByteOffset::new(ack.output_offset),
            ),
            (intent, AckEpoch::new(ack.ack_epoch)),
        );
        match &evidence.write {
            Some(previous) if previous != &write => return Err(OperationError::EvidenceMismatch),
            Some(_) => return Ok(row.entry.record),
            None => evidence.write = Some(write),
        }
        self.update(row)
    }

    /// Adds holder exit facts without changing delivery into a claim of termination.
    ///
    /// An unavailable exit status remains `observed: false`; PTY failure stays explicit.
    ///
    /// # Errors
    ///
    /// Returns `stale_target` for a foreign process or contradictory exit evidence.
    pub fn holder_exit(&self, op: &OpId, exit: &Exit) -> Result<OpRecord, OperationError> {
        let mut row = self.current(op)?;
        let evidence = row
            .entry
            .record
            .evidence
            .as_mut()
            .ok_or(OperationError::EvidenceMismatch)?;
        if !matches_target(evidence, &exit.target) {
            return Err(OperationError::EvidenceMismatch);
        }
        let mut observed =
            ProcessExitEvidence::new((exit.code, exit.signal), ByteOffset::new(exit.final_offset));
        observed.observed &= exit.exit_unknown.is_none();
        observed.io_failure = exit.io_failure.is_some();
        match &evidence.exit {
            Some(previous) if previous != &observed => {
                return Err(OperationError::EvidenceMismatch);
            }
            Some(_) => return Ok(row.entry.record),
            None => evidence.exit = Some(observed),
        }
        self.update(row)
    }
}
