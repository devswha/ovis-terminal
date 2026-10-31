//! Routing of the terminal lifecycle mutations onto the runtime actor.

use std::sync::Arc;

use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_protocol::schema::methods::terminal::{
    TerminalGetParams, TerminalKeepParams, TerminalKillParams, TerminalLabelParams,
    TerminalMutateParams, TerminalRenameParams,
};
use ovis_terminal_server::conn::Call;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::handler::Served;

fn decode<P: DeserializeOwned>(call: &Call) -> Result<P, BoundaryError> {
    decode_params(&call.request).map_err(BoundaryError::from)
}

impl Served {
    /// Runs one lifecycle mutation (or process inspection) on the actor thread.
    pub(super) async fn mutate(&self, call: Call) -> Result<Value, BoundaryError> {
        let session = Arc::clone(&call.session);
        match call.request.method() {
            "terminal.kill" => {
                let params: TerminalKillParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.kill(&session, &params))
                    .await
            }
            "terminal.rm" => {
                let params: TerminalMutateParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.remove(&session, &params))
                    .await
            }
            "terminal.recover" => {
                let params: TerminalMutateParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.recover(&session, &params))
                    .await
            }
            "terminal.keep" => {
                let params: TerminalKeepParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.keep(&session, &params))
                    .await
            }
            "terminal.rename" => {
                let params: TerminalRenameParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.rename(&session, &params))
                    .await
            }
            "terminal.label" => {
                let params: TerminalLabelParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.label(&session, &params))
                    .await
            }
            _ => {
                let params: TerminalGetParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.process_info(&session, &params.target))
                    .await
            }
        }
    }
}
