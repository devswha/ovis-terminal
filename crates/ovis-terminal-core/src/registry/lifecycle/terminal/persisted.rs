//! Persisted identity state must satisfy the same invariants as lifecycle transitions.

use serde::Deserialize;

use super::{
    LifecycleTransitionError, ProcessIdentity, RuntimeIdentity, TerminalLifecycle, TerminalState,
};
use crate::registry::{ProcessGeneration, TerminalGeneration, TerminalId};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PersistedLifecycle {
    identity: RuntimeIdentity,
    id: TerminalId,
    generation: TerminalGeneration,
    state: TerminalState,
    current_process: Option<ProcessIdentity>,
    last_process_generation: Option<ProcessGeneration>,
    previous_process: Option<ProcessIdentity>,
}

impl TryFrom<PersistedLifecycle> for TerminalLifecycle {
    type Error = LifecycleTransitionError;

    fn try_from(raw: PersistedLifecycle) -> Result<Self, Self::Error> {
        let valid = match raw.state {
            TerminalState::Running => raw.current_process.as_ref().is_some_and(|current| {
                Some(current.generation()) == raw.last_process_generation
                    && raw.previous_process.as_ref().is_none_or(|previous| {
                        previous.generation() < current.generation()
                            && (previous.pid() != current.pid()
                                || previous.start_token() != current.start_token())
                    })
            }),
            TerminalState::Lost | TerminalState::Finished => {
                raw.current_process.is_none()
                    && raw.previous_process.as_ref().is_some_and(|previous| {
                        Some(previous.generation()) == raw.last_process_generation
                    })
            }
            TerminalState::RestoredMetadata => {
                raw.current_process.is_none()
                    && raw
                        .previous_process
                        .as_ref()
                        .map(ProcessIdentity::generation)
                        == raw.last_process_generation
            }
        };
        if !valid
            || raw
                .last_process_generation
                .is_some_and(|generation| generation.get() > raw.generation.get())
        {
            return Err(LifecycleTransitionError::InvalidTransition);
        }
        Ok(Self {
            identity: raw.identity,
            id: raw.id,
            generation: raw.generation,
            state: raw.state,
            current_process: raw.current_process,
            last_process_generation: raw.last_process_generation,
            previous_process: raw.previous_process,
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::registry::{
        HostBootId, HostIdentity, LifecycleEvent, MachineId, ProcessHandle, ProcessId, ProcessPid,
        ProcessStartToken, RelaunchEvidence, RuntimeId, ServerBootId,
    };

    fn running(generation: u64, pid: u64) -> TerminalLifecycle {
        let identity = RuntimeIdentity::new(
            HostIdentity::new(
                MachineId::new(uuid::Uuid::from_u128(1)).expect("machine"),
                HostBootId::try_from(1).expect("host"),
            ),
            RuntimeId::try_from(1).expect("runtime"),
            ServerBootId::try_from(1).expect("boot"),
        );
        let metadata = TerminalLifecycle::new(
            identity,
            TerminalId::try_from(1).expect("terminal"),
            TerminalGeneration::new(generation).expect("generation"),
        );
        metadata
            .transition(LifecycleEvent::InitialSpawn {
                evidence: RelaunchEvidence::SpawnConfirmed,
                process: ProcessIdentity::new(
                    ProcessId::try_from(1).expect("process"),
                    ProcessGeneration::new(1).expect("generation"),
                    ProcessHandle::new(
                        ProcessPid::try_from(pid).expect("PID"),
                        ProcessStartToken::new("start").expect("token"),
                    ),
                ),
            })
            .expect("initial spawn")
    }

    #[test]
    fn contradictory_live_metadata_is_rejected_instead_of_claiming_survival() {
        // Given: a reachable live state serialized with a maximum terminal generation.
        let live = running(u64::MAX, u64::MAX);
        let mut encoded = serde_json::to_value(&live).expect("state");
        assert_eq!(
            serde_json::from_value::<TerminalLifecycle>(encoded.clone()).expect("round trip"),
            live
        );

        // When: a stored record loses its current process but still claims Running.
        *encoded.get_mut("current_process").expect("process field") = serde_json::Value::Null;

        // Then: the boundary rejects the contradictory record.
        assert!(serde_json::from_value::<TerminalLifecycle>(encoded).is_err());
    }

    proptest! {
        #[test]
        fn lifecycle_store_round_trips_preserve_live_and_ended_evidence(
            generation in 1..=u64::MAX,
            pid in 1..=u64::MAX,
        ) {
            let live = running(generation, pid);
            let ended = live.transition(LifecycleEvent::ProcessEnded).expect("actual exit");
            for record in [live, ended] {
                let bytes = serde_json::to_vec(&record).expect("encode state");
                prop_assert_eq!(serde_json::from_slice::<TerminalLifecycle>(&bytes).expect("decode state"), record);
            }
        }
    }
}
