//! The typed refusals of a remote request, each naming the machine it was for.

use ovis_terminal_protocol::errors::{BoundaryError, ErrorData, ErrorKind, OpId, TargetId};
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_remote::actors::ActorError;
use serde_json::Value;

use crate::dispatch::lanes::machine_actor::actor_refusal;

pub(super) fn actor(error: &ActorError) -> BoundaryError {
    actor_refusal(error)
}

fn at(refusal: BoundaryError, machine: &MachineId, seq: Option<EventSeq>) -> BoundaryError {
    let refusal = refusal.with_target(TargetId::new(machine.as_str().to_owned()));
    match seq {
        Some(seq) => refusal.with_last_seen_seq(seq),
        None => refusal,
    }
}

/// The machine has no line: nothing was sent and nothing ran locally instead.
pub(super) fn unreachable(machine: &MachineId, seq: Option<EventSeq>) -> BoundaryError {
    let lost = ActorError::Unreachable {
        machine: machine.clone(),
        last_seen_seq: seq,
    };
    at(actor_refusal(&lost), machine, seq)
}

/// The line was lost after the mutation was sent; the machine may have applied it.
pub(super) fn outcome_unknown(
    machine: &MachineId,
    op: OpId,
    seq: Option<EventSeq>,
) -> BoundaryError {
    let unknown = BoundaryError::domain(ErrorKind::from("outcome_unknown".to_owned()), false)
        .with_hint("inspect the operation on the machine once it is reachable before retrying")
        .with_op_id(op);
    at(unknown, machine, seq)
}

/// A refusal that may have begun an effect carries the operation it belongs to.
pub(super) fn with_op(refusal: BoundaryError, op: Option<OpId>) -> BoundaryError {
    match op {
        Some(op) => refusal.with_op_id(op),
        None => refusal,
    }
}

/// The machine's own typed refusal, kept as it answered.
pub(super) fn answered(error: &Value) -> BoundaryError {
    error
        .get("data")
        .cloned()
        .and_then(|data| serde_json::from_value::<ErrorData>(data).ok())
        .map_or_else(
            || {
                BoundaryError::domain(ErrorKind::from("internal".to_owned()), false)
                    .with_hint("the machine answered with an error this build cannot read")
            },
            |data| BoundaryError::Domain {
                data: Box::new(data),
            },
        )
}

/// No machine of this runtime answers to the qualifier; nothing is served locally instead.
pub(super) fn unknown_machine(field: &str) -> BoundaryError {
    BoundaryError::domain(ErrorKind::from("not_found".to_owned()), false)
        .with_hint("list machines to find a live name; nothing ran locally instead")
        .with_field_path(field)
}
