//! The blocking waits (`turn.wait`, `events.wait`, `input.interrupt`): armed on the actor,
//! awaited off it.

use std::sync::Arc;

use ovis_terminal_core::waits::Ticket;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::methods::events::{EventsWaitParams, EventsWaitResult};
use ovis_terminal_protocol::schema::methods::input::{InputInterruptParams, InputInterruptResult};
use ovis_terminal_protocol::schema::methods::status::{TurnRecord, TurnWaitParams};
use ovis_terminal_server::conn::Call;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::error::{internal, worker_gone};
use super::handler::{Served, decode};
use super::turns::{Armed, state_outcome, turn_record};

/// Awaits `armed` off the actor thread; its guard is released once it resolved.
async fn settled<T: Send + 'static>(armed: Armed<T>) -> Result<T, BoundaryError> {
    let (ticket, deadline, guard): (Ticket<T>, _, _) = armed;
    tokio::task::spawn_blocking(move || {
        let outcome = ticket.wait(deadline);
        drop(guard);
        outcome
    })
    .await
    .map_err(|_| worker_gone())?
    .map_err(|error| internal("await a wait")(error))
}

fn checked<T: DeserializeOwned + serde::Serialize>(value: Value) -> Result<Value, BoundaryError> {
    let typed: T = serde_json::from_value(value).map_err(internal("encode a wait result"))?;
    serde_json::to_value(typed).map_err(internal("encode a wait result"))
}

impl Served {
    /// `turn.wait`, `events.wait` and `input.interrupt`.
    pub(super) async fn waited(&self, call: Call) -> Result<Value, BoundaryError> {
        match call.request.method() {
            "turn.wait" => {
                let params: TurnWaitParams = decode(&call)?;
                let send_op = params.send_op.clone();
                let armed = self
                    .actor
                    .call(move |runtime| runtime.arm_turn_wait(&params))
                    .await?;
                let outcome = settled(armed).await?;
                checked::<TurnRecord>(turn_record(&outcome, send_op.as_ref()))
            }
            "events.wait" => {
                let params: EventsWaitParams = decode(&call)?;
                let again = params.clone();
                let (armed, _) = self
                    .actor
                    .call(move |runtime| runtime.arm_state_wait(&params))
                    .await?;
                let outcome = settled(armed).await?;
                // Read after settling: a wait that matched later reports the status it matched.
                let seq = self
                    .actor
                    .call(move |runtime| runtime.status_seq(&again))
                    .await?;
                let seq = seq.map_or_else(|| "0".to_owned(), |seq| seq.get().to_string());
                checked::<EventsWaitResult>(
                    json!({"outcome": state_outcome(&outcome)?, "seq": seq}),
                )
            }
            _ => {
                let params: InputInterruptParams = decode(&call)?;
                let session = Arc::clone(&call.session);
                let (armed, receipt) = self
                    .actor
                    .call(move |runtime| runtime.arm_interrupt(&session, &params))
                    .await?;
                let observed = settled(armed).await?;
                let written = super::input::result(&receipt)?;
                checked::<InputInterruptResult>(json!({
                    "receipt": written.get("receipt"), "turn": observed.turn,
                    "stopped": observed.stopped(),
                }))
            }
        }
    }
}
