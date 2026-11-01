//! `input.barrier`: where the input admitted up to one position stands, read in holder order
//! on the actor thread. It waits for nothing and writes nothing.

use ovis_terminal_core::input::receipts::InputSeq;
use ovis_terminal_core::input::sequencer::BarrierState;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::methods::input::{InputBarrierParams, InputBarrierResult};
use serde_json::{Value, json};

use super::actor::Runtime;
use super::error::{checked, internal, refusal};
use super::input::live_terminal;

impl Runtime {
    /// Whether every byte admitted up to `params.queued_seq` reached the program, at the
    /// raw output offset of the moment it was evaluated.
    pub(super) fn barrier(&mut self, params: &InputBarrierParams) -> Result<Value, BoundaryError> {
        let live = live_terminal((&self.directory, &mut self.live), &params.target)?;
        let state = live
            .sequencer
            .barrier(InputSeq::new(params.queued_seq.get()))
            .map_err(|error| refusal(error.kind(), error.hint()))?;
        let offset = live
            .hub
            .screen()
            .live()
            .map_err(internal("read the raw output offset"))?
            .offset
            .get();
        let (reached, pending) = match state {
            BarrierState::Reached => (true, 0),
            BarrierState::Pending { pending_bytes } => (false, pending_bytes.get()),
            BarrierState::Undelivered { .. } => (false, 0),
            _ => {
                return Err(refusal(
                    "internal",
                    "report the build: an unknown barrier state",
                ));
            }
        };
        checked::<InputBarrierResult>(json!({
            "reached": reached,
            "output_offset": offset.to_string(),
            "pending_bytes": pending.to_string(),
        }))
    }
}
