//! Interrupted operations, classified from the committed ledger and holder evidence only.

use ovis_terminal_protocol::domain::restart::{Delivery, InterruptedOp, InterruptedOutcome};
use ovis_terminal_protocol::schema::methods::server::{OpRecord, OpState};
use ovis_terminal_protocol::schema::scalar::OpenEnum;

/// How `record` ended at a restart boundary.
///
/// Only a ledger commit is `committed`. An operation is `not_started` only when it was
/// never started and no write intent or acknowledgement exists; anything that may have
/// reached the holder is `outcome_unknown`, carrying the holder's write acknowledgement
/// when one was recorded. A state this build does not know is never read as success.
#[must_use]
pub fn classify(record: &OpRecord) -> InterruptedOp {
    let evidence = record.evidence.as_ref();
    let delivery = evidence
        .and_then(|evidence| evidence.write.as_ref())
        .map_or(Delivery::Unproven {}, |write| Delivery::Written {
            written_bytes: write.written_bytes,
            output_offset: write.output_offset,
        });
    let touched =
        evidence.is_some_and(|evidence| evidence.intent.is_some() || evidence.write.is_some());
    let outcome = match record.state {
        OpenEnum::Known(OpState::Committed) => InterruptedOutcome::Committed {},
        OpenEnum::Known(OpState::Failed) => InterruptedOutcome::Failed {},
        OpenEnum::Known(OpState::Cancelled) => InterruptedOutcome::Cancelled {},
        OpenEnum::Known(OpState::Accepted) if !touched => InterruptedOutcome::NotStarted {},
        OpenEnum::Known(_) | OpenEnum::Unknown(_) => {
            InterruptedOutcome::OutcomeUnknown { delivery }
        }
    };
    InterruptedOp::new(record.op_id.clone(), outcome)
}

#[cfg(test)]
mod tests {
    use ovis_terminal_protocol::errors::OpId;
    use ovis_terminal_protocol::schema::ids::{MethodName, TerminalId};
    use ovis_terminal_protocol::schema::methods::server::{
        AckEpoch, FenceEpoch, GrantEpoch, OperationEvidence, WriteEvidence, WriteIntent,
        WriteSequence,
    };
    use ovis_terminal_protocol::schema::scalar::{ByteCount, ByteOffset, Incarnation};
    use proptest::prelude::*;

    use super::*;

    const STATES: [OpState; 6] = [
        OpState::Accepted,
        OpState::Running,
        OpState::Committed,
        OpState::Failed,
        OpState::Cancelled,
        OpState::OutcomeUnknown,
    ];

    fn record(state: OpState, (intent, written): (bool, Option<(u64, u64)>)) -> OpRecord {
        let mut record = OpRecord::accepted(
            OpId::new("op".to_owned()),
            MethodName::new("input.send".to_owned()),
        );
        record.state = OpenEnum::Known(state);
        if intent || written.is_some() {
            let mut evidence = OperationEvidence::for_process(
                TerminalId::new("terminal:4".to_owned()),
                (Incarnation::new(1), Some(Incarnation::new(1))),
            );
            let bound = WriteIntent::new(
                WriteSequence::new(1),
                GrantEpoch::new(1),
                FenceEpoch::new(1),
            );
            evidence.intent = intent.then(|| bound.clone());
            evidence.write = written.map(|(bytes, offset)| {
                WriteEvidence::new(
                    (ByteCount::new(bytes), ByteOffset::new(offset)),
                    (bound, AckEpoch::new(1)),
                )
            });
            record.evidence = Some(evidence);
        }
        record
    }

    proptest! {
        #[test]
        fn only_the_ledger_commits_and_touched_work_is_never_not_started(
            state in prop::sample::select(STATES.to_vec()),
            intent in any::<bool>(),
            written in prop::option::of((any::<u64>(), any::<u64>())),
        ) {
            let outcome = classify(&record(state, (intent, written))).outcome;
            prop_assert_eq!(outcome == InterruptedOutcome::Committed {}, state == OpState::Committed);
            if intent || written.is_some() {
                prop_assert_ne!(outcome, InterruptedOutcome::NotStarted {});
            }
            if let InterruptedOutcome::OutcomeUnknown { delivery } = outcome {
                let expected = written.map_or(Delivery::Unproven {}, |(bytes, offset)| Delivery::Written {
                    written_bytes: ByteCount::new(bytes),
                    output_offset: ByteOffset::new(offset),
                });
                prop_assert_eq!(delivery, expected);
            }
        }
    }

    #[test]
    fn an_unacknowledged_running_write_stays_outcome_unknown_and_an_unknown_state_is_not_success() {
        let running = classify(&record(OpState::Running, (true, None)));
        assert_eq!(
            running.outcome,
            InterruptedOutcome::OutcomeUnknown {
                delivery: Delivery::Unproven {}
            }
        );
        assert!(running.uncertain());
        let mut future = record(OpState::Committed, (false, None));
        future.state = OpenEnum::Unknown("settled_elsewhere".to_owned());
        assert!(classify(&future).uncertain());
        assert_eq!(
            classify(&record(OpState::Accepted, (false, None))).outcome,
            InterruptedOutcome::NotStarted {}
        );
    }
}
