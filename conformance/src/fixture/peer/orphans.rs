//! Orphan notices between the two runtimes: `$/orphaned` received by a served runtime is
//! applied by its machine actor lane, and the sending side reports what it still owes.

use ovis_terminal_protocol::domain::machine_actor::OrphanedParams;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_remote::orphans::{OrphanError, Replayed};
use ovis_terminal_server::dispatch::lanes::machine_actor::orphan_refusal;
use serde_json::{Value, json};

use super::super::machines::{Lane, Machines, internal};

/// `$/orphaned`: the notice is applied once by this runtime's lane and acknowledged.
pub(crate) async fn orphaned(
    machines: &Machines,
    request: &RpcRequest,
) -> Result<Value, BoundaryError> {
    let params: OrphanedParams = decode_params(request).map_err(BoundaryError::from)?;
    let (result, end_orders) = machines
        .lane()
        .orphaned(&params)
        .await
        .map_err(|error| orphan_refusal(&error))?;
    if !end_orders.is_empty() {
        return Err(internal(
            "a die-with-parent child needs the remote launch route",
        ));
    }
    serde_json::to_value(result).map_err(internal)
}

/// The replay outcome and the notices still owed to `machine`.
pub(super) async fn report(
    lane: &Lane,
    machine: &MachineId,
    replayed: Result<Replayed, OrphanError>,
) -> Result<Value, BoundaryError> {
    let replayed = replayed.map_err(|error| orphan_refusal(&error))?;
    let outbox = lane
        .pending(machine)
        .await
        .map_err(|error| orphan_refusal(&error))?;
    Ok(json!({
        "replayed": {
            "applied": replayed.applied, "duplicates": replayed.duplicates,
            "remaining": replayed.remaining,
            "stopped": replayed.stopped.as_ref().map(OrphanError::kind),
        },
        "outbox": outbox,
    }))
}
