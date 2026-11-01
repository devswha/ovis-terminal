//! Process-token authentication of `prompt.open` and `prompt.close` (plan todo 40).
//!
//! A program authenticates these requests with the token handed to it at spawn. The gate
//! reads only the credential fields and refuses, as `forbidden`, a token that names no
//! current program, or a target or prompt on another program's terminal, before the
//! closed parameter type is decoded or the operation admitted. Requests it passes go on
//! unchanged to the prompt lane, which checks the token again inside its commit unit.

use std::sync::Arc;

use ovis_terminal_agent::bridge::BridgeError;
use ovis_terminal_agent::prompts::PromptService;
use ovis_terminal_agent::status::StatusService;
use ovis_terminal_protocol::domain::prompt_bridge::ProgramCredential;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::limits::TransportKind;
use serde::Deserialize as _;
use serde_json::Value;

use super::agent_status::TerminalDirectory;
use crate::conn::{Call, RequestHandler};

/// Methods a program authenticates with its process token.
pub const GATED: [&str; 2] = ["prompt.open", "prompt.close"];

/// Refuses program-authenticated prompt requests whose token does not own their target.
#[derive(Debug)]
pub struct PromptBridgeGate<T> {
    terminals: T,
    status: Arc<StatusService>,
    prompts: Arc<PromptService>,
}

impl<T: TerminalDirectory> PromptBridgeGate<T> {
    /// Checks tokens against `status`, targets through `terminals` and prompts through
    /// `prompts`' routes.
    #[must_use]
    pub fn new(terminals: T, (status, prompts): (Arc<StatusService>, Arc<PromptService>)) -> Self {
        Self {
            terminals,
            status,
            prompts,
        }
    }

    /// Admits every request except a gated one whose token does not own what it names.
    ///
    /// A gated request without a readable credential passes, so the closed decoder
    /// reports its typed `invalid_params`.
    ///
    /// # Errors
    ///
    /// Returns `forbidden` with no operation admitted and no prompt read or changed.
    pub fn admit(&self, request: &RpcRequest) -> Result<(), BoundaryError> {
        if !GATED.contains(&request.method()) {
            return Ok(());
        }
        let Ok(credential) = ProgramCredential::deserialize(request.params()) else {
            return Ok(());
        };
        self.check(&credential)
            .map_err(|error| BoundaryError::Domain {
                data: Box::new(error.error_data()),
            })
    }

    fn check(&self, credential: &ProgramCredential) -> Result<(), BridgeError> {
        let owner = self
            .status
            .tokens()
            .owner(&credential.process_token)
            .map_err(BridgeError::Owner)?
            .ok_or(BridgeError::Forbidden)?;
        if let Some(target) = &credential.target
            && let Ok(resolved) = self.terminals.resolve(target)
            && resolved.terminal != owner.terminal
        {
            return Err(BridgeError::Forbidden);
        }
        if let Some(prompt) = &credential.prompt_id
            && let Ok(routed) = self.prompts.route(prompt)
            && routed != owner.terminal
        {
            return Err(BridgeError::Forbidden);
        }
        Ok(())
    }
}

/// A request handler that runs the gate before every call of `inner`.
#[derive(Debug)]
pub struct Gated<H, T> {
    gate: PromptBridgeGate<T>,
    inner: Arc<H>,
}

impl<H, T> Gated<H, T> {
    /// Serves `inner` behind `gate`.
    #[must_use]
    pub const fn new(gate: PromptBridgeGate<T>, inner: Arc<H>) -> Self {
        Self { gate, inner }
    }
}

impl<H: RequestHandler, T: TerminalDirectory> RequestHandler for Gated<H, T> {
    fn methods(&self) -> Vec<&'static str> {
        self.inner.methods()
    }

    fn transports(&self) -> Vec<TransportKind> {
        self.inner.transports()
    }

    async fn call(&self, call: Call) -> Result<Value, BoundaryError> {
        self.gate.admit(&call.request)?;
        self.inner.call(call).await
    }
}

#[cfg(test)]
mod qa;
