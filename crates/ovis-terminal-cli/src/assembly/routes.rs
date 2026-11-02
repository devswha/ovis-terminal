//! Routing of the terminal lifecycle mutations onto the runtime actor.

use std::sync::Arc;

use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_protocol::schema::methods::input::{
    InputKeysParams, InputPasteParams, InputRawParams, InputSendParams,
};
use ovis_terminal_protocol::schema::methods::terminal::{
    TerminalGetParams, TerminalKeepParams, TerminalKillParams, TerminalLabelParams,
    TerminalMutateParams, TerminalRenameParams,
};
use ovis_terminal_protocol::schema::methods::terminal_read::{
    TerminalOutputParams, TerminalScreenParams, TerminalSearchParams, TerminalWaitOutputParams,
};
use ovis_terminal_server::conn::Call;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::error::{internal, worker_gone};
use super::handler::Served;
use super::reads::waited;

fn decode<P: DeserializeOwned>(call: &Call) -> Result<P, BoundaryError> {
    decode_params(&call.request).map_err(BoundaryError::from)
}

impl Served {
    /// Admits one `input.*` write on the actor thread.
    pub(super) async fn write(&self, call: Call) -> Result<Value, BoundaryError> {
        let session = Arc::clone(&call.session);
        match call.request.method() {
            "input.raw" => {
                let params: InputRawParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.input_raw(&session, &params))
                    .await
            }
            "input.send" => {
                let params: InputSendParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.input_send(&session, &params))
                    .await
            }
            "input.keys" => {
                let params: InputKeysParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.input_keys(&session, &params))
                    .await
            }
            _ => {
                let params: InputPasteParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.input_paste(&session, &params))
                    .await
            }
        }
    }

    /// Serves a screen, search, output or output-wait read; a wait blocks off the actor thread.
    pub(super) async fn read(&self, call: Call) -> Result<Value, BoundaryError> {
        match call.request.method() {
            "terminal.screen" => {
                let params: TerminalScreenParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.screen(&params))
                    .await
            }
            "terminal.search" => {
                let params: TerminalSearchParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.search(&params))
                    .await
            }
            "terminal.output" => {
                let params: TerminalOutputParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.output(&params))
                    .await
            }
            _ => {
                let params: TerminalWaitOutputParams = decode(&call)?;
                let (ticket, deadline, armed) = self
                    .actor
                    .call(move |runtime| runtime.arm_output_wait(&params))
                    .await?;
                let outcome = tokio::task::spawn_blocking(move || {
                    let outcome = ticket.wait(deadline);
                    drop(armed);
                    outcome
                })
                .await
                .map_err(|_| worker_gone())?
                .map_err(|error| internal("await an output wait")(error))?;
                waited(&outcome)
            }
        }
    }

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
