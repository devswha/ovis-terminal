//! `fixture.lost_machine`: a machine whose line mirrored events through a known sequence
//! and then ended, as a disconnected machine is after its runtime went away.

use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_protocol::schema::ids::{MachineId, MachineName};
use ovis_terminal_protocol::schema::methods::machine::MachineState;
use ovis_terminal_protocol::schema::scalar::{EventSeq, OpenEnum};
use ovis_terminal_remote::actors::Lines;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt as _;

use super::super::machines::{FixtureLine, Machines, internal};
use super::BOUND;

/// The unadvertised method that adds a lost machine.
pub(crate) const LOST_MACHINE: &str = "fixture.lost_machine";

/// Identity of the lost machine.
const LOST: &str = "machine:0191f1a2-0000-7000-8000-0000000000d4";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LostParams {
    name: MachineName,
    last_seen_seq: EventSeq,
}

/// Adds machine `name`, lets its line deliver one event at `last_seen_seq` and end, and
/// returns once its actor reports it unreachable at that sequence.
pub(crate) async fn lost_machine(
    machines: &Machines,
    request: &RpcRequest,
) -> Result<Value, BoundaryError> {
    let params: LostParams = decode_params(request).map_err(BoundaryError::from)?;
    let machine = MachineId::new(LOST.to_owned());
    let record = serde_json::from_value(json!({
        "machine_id": machine, "name": params.name, "aliases": [], "state": "connecting",
        "remote_lifetime": "runtime:9", "remote_boot_id": "1", "last_seen_seq": null,
        "version": "1.0.0", "revision": "1",
    }))
    .map_err(internal)?;
    let actors = machines.lane().actors();
    actors.add(record).map_err(internal)?;
    let mut health = actors.watch_health(&machine).map_err(internal)?;
    let (local, mut remote) = tokio::io::duplex(64 * 1024);
    let event = json!({"jsonrpc": "2.0", "method": "event", "params": {
        "seq": params.last_seen_seq, "runtime_lifetime": "9", "ts_ms": "1",
        "type": "terminal.created", "terminal": null, "data": {},
    }});
    remote
        .write_all(format!("{event}\n").as_bytes())
        .await
        .map_err(internal)?;
    drop(remote);
    let (read, write) = tokio::io::split(local);
    let line = FixtureLine::First(Lines::new(read, write));
    actors.attach(&machine, line).await.map_err(internal)?;
    let unreachable = OpenEnum::Known(MachineState::Unreachable);
    let seen = Some(params.last_seen_seq);
    let lost =
        health.wait_for(|health| health.state == unreachable && health.last_seen_seq == seen);
    tokio::time::timeout(BOUND, lost)
        .await
        .map_err(internal)?
        .map_err(internal)?;
    let health = actors.health(&machine).map_err(internal)?;
    serde_json::to_value(health).map_err(internal)
}
