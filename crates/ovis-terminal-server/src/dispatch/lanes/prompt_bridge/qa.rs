//! The gate refuses foreign process tokens before decoding and passes everything else on.

use std::error::Error;
use std::sync::Arc;

use ovis_terminal_agent::bridge::BridgeError;
use ovis_terminal_agent::prompts::PromptService;
use ovis_terminal_agent::status::{ProcessOwner, StatusService};
use ovis_terminal_core::registry::{
    MachineId, RuntimeId, TargetResolutionError, TerminalGeneration, TerminalId,
};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::rpc::parse_request;
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::ids::ProcessToken;
use ovis_terminal_protocol::schema::scalar::Incarnation;
use serde_json::{Value, json};

use super::PromptBridgeGate;
use crate::dispatch::lanes::agent_status::{ResolvedTerminal, TerminalDirectory};

type TestResult = Result<(), Box<dyn Error>>;

/// Terminals 4 (`agent`) and 5 (`shell`) of one machine.
#[derive(Debug)]
struct Two(MachineId);

impl TerminalDirectory for Two {
    fn resolve(&self, target: &Target) -> Result<ResolvedTerminal, TargetResolutionError> {
        let id = match target {
            Target::Name(name) if name.as_str() == "agent" => 4,
            Target::Name(name) if name.as_str() == "shell" => 5,
            _ => return Err(TargetResolutionError::NotFound),
        };
        let terminal = TerminalId::try_from(id).map_err(|_| TargetResolutionError::NotFound)?;
        let generation = TerminalGeneration::new(1).map_err(|_| TargetResolutionError::NotFound)?;
        Ok(ResolvedTerminal::new(self.0, terminal, generation))
    }
}

fn gate() -> Result<PromptBridgeGate<Two>, Box<dyn Error>> {
    let machine: MachineId = "machine:0191f1a2-0000-7000-8000-000000000040".parse()?;
    let runtime = RuntimeId::try_from(40)?;
    let status = Arc::new(StatusService::new(runtime));
    for (id, token) in [(4, "token-agent"), (5, "token-shell")] {
        let owner = ProcessOwner::new(TerminalId::try_from(id)?, Incarnation::new(1));
        status
            .tokens()
            .bind(&ProcessToken::new(token.to_owned()), owner)?;
    }
    let prompts = Arc::new(PromptService::new(runtime, Arc::clone(&status)));
    Ok(PromptBridgeGate::new(Two(machine), (status, prompts)))
}

fn admit(gate: &PromptBridgeGate<Two>, method: &str, params: &Value) -> Result<(), BoundaryError> {
    let request = json!({"jsonrpc": "2.0", "id": "1", "method": method, "params": params});
    gate.admit(&parse_request(&request.to_string())?)
}

fn kind(outcome: Result<(), BoundaryError>) -> Option<String> {
    outcome.err().map(|error| error.kind().to_owned())
}

#[test]
fn a_foreign_token_is_forbidden_before_the_prompt_is_decoded() -> TestResult {
    let gate = gate()?;
    let injected = json!({"op_id": "o", "process_token": "other-process",
        "prompt": {"kind": "form", "mechanism": "extension"}});
    let refused = admit(&gate, "prompt.open", &injected)
        .err()
        .ok_or("admitted")?;
    assert_eq!(refused.kind(), "forbidden");
    assert_eq!(refused.data().hint, BridgeError::Forbidden.hint());
    let close = json!({"op_id": "o", "process_token": "other-process",
        "prompt_id": "p1", "outcome": "dismissed"});
    assert_eq!(
        kind(admit(&gate, "prompt.close", &close)).as_deref(),
        Some("forbidden")
    );
    Ok(())
}

#[test]
fn a_token_naming_another_programs_terminal_is_forbidden() -> TestResult {
    let gate = gate()?;
    let cross = json!({"process_token": "token-shell", "target": {"name": "agent"},
        "prompt": {"kind": "form", "mechanism": "extension"}});
    assert_eq!(
        kind(admit(&gate, "prompt.open", &cross)).as_deref(),
        Some("forbidden")
    );
    let own = json!({"process_token": "token-agent", "target": {"name": "agent"},
        "prompt": {"kind": "form", "mechanism": "extension"}});
    assert_eq!(kind(admit(&gate, "prompt.open", &own)), None);
    Ok(())
}

#[test]
fn ungated_and_credential_less_requests_reach_the_strict_decoder() -> TestResult {
    let gate = gate()?;
    let answer = json!({"process_token": "other-process", "prompt_id": "p1"});
    assert_eq!(kind(admit(&gate, "prompt.answer", &answer)), None);
    let tokenless = json!({"prompt": {"kind": "form"}});
    assert_eq!(kind(admit(&gate, "prompt.open", &tokenless)), None);
    let numeric = json!({"process_token": 7});
    assert_eq!(kind(admit(&gate, "prompt.open", &numeric)), None);
    Ok(())
}
