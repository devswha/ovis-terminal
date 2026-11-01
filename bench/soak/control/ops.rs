//! The operation journal of the lost-receipt fixture: an input's intent is durable
//! before the holder sees it, and a boot after a crash classifies what it finds from the
//! journal alone. Nothing here ever sends the input again.

use std::path::Path;

use ovis_terminal_core::restart::classify;
use ovis_terminal_holder::protocol::{Input, TerminalTarget, WriteGrant};
use ovis_terminal_protocol::domain::restart::InterruptedOp;
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::{MethodName, TerminalId};
use ovis_terminal_protocol::schema::methods::server::{
    FenceEpoch, GrantEpoch, OpRecord, OpState, OperationEvidence, WriteIntent, WriteSequence,
};
use ovis_terminal_protocol::schema::scalar::{ByteCount, Count, Incarnation, OpenEnum};

use super::super::error::SoakError;
use super::super::wire::Event;
use super::Control;
use super::links::Outbound;
use super::setup::{read, write_atomic};

const OPS: &str = "ops.json";
/// Bytes the lost-receipt fixture writes to terminal 0.
const LOST_INPUT: &[u8] = b"soak-lost-receipt\r";

/// Records `input.send` of `bytes` to `target` as running with its write intent.
pub(super) fn intend(
    state: &Path,
    (target, sequence): (&TerminalTarget, u64),
    bytes: usize,
) -> Result<OpRecord, SoakError> {
    let op_id = OpId::new(format!(
        "op:soak-input-{}-{sequence}",
        target.terminal_id.as_str()
    ));
    let mut record = OpRecord::accepted(op_id, MethodName::new("input.send".to_owned()));
    record.state = OpenEnum::Known(OpState::Running);
    let mut evidence = OperationEvidence::for_process(
        TerminalId::new(target.terminal_id.as_str().to_owned()),
        (
            Incarnation::new(target.terminal_incarnation.get()),
            target
                .process_incarnation
                .map(|incarnation| Incarnation::new(incarnation.get())),
        ),
    );
    let length = u64::try_from(bytes).unwrap_or(u64::MAX);
    evidence.intent = Some(
        WriteIntent::new(
            WriteSequence::new(sequence),
            GrantEpoch::new(1),
            FenceEpoch::new(1),
        )
        .with_bytes(ByteCount::new(length)),
    );
    record.evidence = Some(evidence);
    let mut journal = journal(state)?;
    journal.push(record.clone());
    write_atomic(&state.join(OPS), &serde_json::to_vec_pretty(&journal)?)?;
    Ok(record)
}

fn journal(state: &Path) -> Result<Vec<OpRecord>, SoakError> {
    if state.join(OPS).exists() {
        read(&state.join(OPS))
    } else {
        Ok(Vec::new())
    }
}

/// How every journalled operation ended at the boundary this boot follows.
pub(super) fn interrupted(state: &Path) -> Result<Vec<InterruptedOp>, SoakError> {
    Ok(journal(state)?.iter().map(classify).collect())
}

impl Control {
    /// Fixture: journals the intent, hands the input to the holder and never commits
    /// its receipt.
    pub(super) fn lose_receipt(&mut self) -> Result<(), SoakError> {
        let Some(screen) = self.screens.first().and_then(Option::as_ref) else {
            return Err(SoakError::Protocol {
                expected: "a live terminal 0 for the fixture input",
            });
        };
        let target = screen.target.clone();
        let process_incarnation = target.process_incarnation.ok_or(SoakError::Protocol {
            expected: "a process incarnation for terminal 0",
        })?;
        let record = intend(&self.setup.state, (&target, 1), LOST_INPUT.len())?;
        let input = Input {
            target,
            sequence: 1,
            grant: WriteGrant {
                grant_epoch: 1,
                fence_epoch: 1,
                process_incarnation,
            },
            bytes: LOST_INPUT.to_vec(),
        };
        let link = self
            .links
            .get(self.setup.group_of(0))
            .and_then(Option::as_ref);
        link.and_then(|link| link.send(Outbound::Input(Box::new(input))).ok())
            .ok_or(SoakError::Protocol {
                expected: "a live link to terminal 0's holder",
            })?;
        self.hub.publish(Event::InputSent {
            terminal: Count::new(0),
            op_id: record.op_id.as_str().to_owned(),
            sequence: Count::new(1),
        })
    }
}
