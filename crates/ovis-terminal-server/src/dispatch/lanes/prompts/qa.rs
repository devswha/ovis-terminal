//! The prompt lane over a real dispatcher, authority, store and socket.
//!
//! Answer permission is checked on the prompt's terminal independently of interact
//! permission, retries are answered from the ledger, and counters stay decimal strings.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_agent::prompts::PromptService;
use ovis_terminal_agent::status::{AgentStatusEvent, ProcessOwner, StatusService};
use ovis_terminal_authz::{
    EndpointPolicy, Grant, Issuance, IssueRequest, LocalAccount, PeerEvidence, Permission,
    Presentation, PrincipalId, RemoteTransport, ResourceScope, ScopeResource,
};
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::OperationProducer;
use ovis_terminal_core::registry::{TargetResolutionError, TerminalGeneration, TerminalId};
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::ids::ProcessToken;
use ovis_terminal_protocol::schema::scalar::{Incarnation, TimestampMs};
use ovis_terminal_server::dispatch::Dispatcher;
use ovis_terminal_server::dispatch::lanes::agent_status::{ResolvedTerminal, TerminalDirectory};
use ovis_terminal_server::dispatch::lanes::prompts::{
    AnswerPrompt, GetPrompt, ListPrompts, OpenPrompt, PromptLane,
};
use ovis_terminal_server::listen::ServerHandle;
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};

use super::support::socket::{self, Client};
use super::support::{TestResult, at, error_kind, request, runtime};

type Dispatch = Dispatcher<AgentStatusEvent, OperationProducer>;
type Core = Coordinator<AgentStatusEvent, OperationProducer>;

const TOKEN: &str = "token-agent";

/// Terminal 4, named `agent` or addressed by its id, at generation 1.
#[derive(Debug)]
struct OneTerminal(ResolvedTerminal);

impl TerminalDirectory for OneTerminal {
    fn resolve(&self, target: &Target) -> Result<ResolvedTerminal, TargetResolutionError> {
        match target {
            Target::Name(name) if name.as_str() == "agent" => Ok(self.0),
            Target::TerminalId(id) if id.as_str() == self.0.terminal.to_string() => Ok(self.0),
            _ => Err(TargetResolutionError::NotFound),
        }
    }
}

struct Lane {
    socket: socket::Fixture,
    core: Arc<Core>,
    dispatch: Arc<Dispatch>,
}

fn lane() -> TestResult<Lane> {
    let socket = socket::Fixture::new()?;
    let identity = runtime(1)?;
    let terminal = TerminalId::try_from(4)?;
    let status = Arc::new(StatusService::new(identity.runtime_id()));
    let owner = ProcessOwner::new(terminal, Incarnation::new(1));
    status
        .tokens()
        .bind(&ProcessToken::new(TOKEN.to_owned()), owner)?;
    let resolved =
        ResolvedTerminal::new(identity.machine_id(), terminal, TerminalGeneration::new(1)?);
    let store = Store::open(&socket.path("db"), StoreOptions::default())?;
    let core = Arc::new(Coordinator::new(
        store,
        identity,
        OperationProducer::new(identity.runtime_id()),
    ));
    let mut dispatch = Dispatch::new(Arc::clone(&socket.authority), identity, Arc::clone(&core));
    let service = Arc::new(PromptService::new(identity.runtime_id(), status));
    let lane = PromptLane::new(OneTerminal(resolved), service, identity.machine_id());
    dispatch.register(OpenPrompt::new(&lane))?;
    dispatch.register(AnswerPrompt::new(&lane))?;
    dispatch.register_read(GetPrompt::new(&lane))?;
    dispatch.register_read(ListPrompts::new(&lane))?;
    Ok(Lane {
        socket,
        core,
        dispatch: Arc::new(dispatch),
    })
}

impl Lane {
    /// A bearer for `principal` holding `permissions` over the whole machine.
    fn issue(&self, principal: &str, permissions: &[Permission]) -> TestResult<String> {
        let policy =
            EndpointPolicy::new(LocalAccount::Unix(ovis_terminal_authz::UnixUid::new(1000)));
        let admission = policy.admit(&PeerEvidence::Remote(RemoteTransport::Ssh))?;
        let authority = &self.socket.authority;
        let elapsed = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let now = TimestampMs::new(u64::try_from(elapsed)?);
        let owner =
            authority.authenticate(&admission, Presentation::new(&self.socket.bearer, now))?;
        let scope = ResourceScope::new(runtime(1)?.machine_id(), ScopeResource::Machine);
        let grant = |permission: &Permission| Grant::new(*permission, scope.clone());
        let grants = permissions.iter().map(grant).collect();
        let request = IssueRequest::new(
            PrincipalId::parse(principal)?,
            grants,
            TimestampMs::new(u64::MAX),
        );
        Ok(authority
            .issue(&owner, Issuance::new(request, now))?
            .bearer()
            .to_owned())
    }

    async fn client(&self, bearer: &str) -> TestResult<Client> {
        let mut client = self.socket.connect().await?;
        client.hello("1", bearer).await?;
        Ok(client)
    }

    async fn close(self, server: ServerHandle) -> TestResult {
        server.shutdown().await?;
        self.dispatch.shutdown().await?;
        drop(self.dispatch);
        let core = Arc::try_unwrap(self.core).map_err(|_| "core still owned")?;
        tokio::task::spawn_blocking(move || core.close()).await??;
        Ok(())
    }
}

async fn ask(client: &mut Client, (id, method): (&str, &str), params: Value) -> TestResult<Value> {
    client.send(&request(id, method, &params)).await?;
    let reply = client.expect_next().await?;
    assert_eq!(at(&reply, "/id"), id, "{reply}");
    Ok(reply)
}

fn op() -> String {
    uuid::Uuid::now_v7().to_string()
}

fn open() -> Value {
    json!({"op_id": op(), "target": {"name": "agent"}, "process_token": TOKEN, "prompt_id": "p1",
           "prompt": {"kind": "permission", "text": "Run tests?", "mechanism": "rpc",
                      "choices": [{"id": "yes", "label": "Yes"}, {"id": "no", "label": "No"}]}})
}

fn answer(op: &str, version: &Value, answer: &Value) -> Value {
    json!({"op_id": op, "prompt_id": "p1", "version": version, "answer": answer})
}

#[tokio::test(flavor = "multi_thread")]
async fn answering_needs_prompt_answer_on_the_terminal_not_interact_or_observe() -> TestResult {
    // Given: p1 open at version 2; a typist (observe + interact), an observer and an answerer.
    let lane = lane()?;
    let typist = lane.issue("typist", &[Permission::Observe, Permission::Interact])?;
    let observer = lane.issue("observer", &[Permission::Observe])?;
    let answerer = lane.issue("answerer", &[Permission::PromptAnswer])?;
    let server = lane.socket.serve(1, Arc::clone(&lane.dispatch)).await?;
    let mut owner = lane.client(&lane.socket.bearer).await?;
    ask(&mut owner, ("open-1", "prompt.open"), open()).await?;
    ask(&mut owner, ("open-2", "prompt.open"), open()).await?;

    // When: each principal answers version 2.
    let mut results = Vec::new();
    for (name, bearer) in [
        ("typist", &typist),
        ("observer", &observer),
        ("answerer", &answerer),
    ] {
        let mut client = lane.client(bearer).await?;
        let params = answer(&op(), &json!("2"), &json!({"choice": "yes"}));
        results.push(ask(&mut client, (name, "prompt.answer"), params).await?);
    }
    let read = ask(
        &mut owner,
        ("get", "prompt.get"),
        json!({"prompt_id": "p1"}),
    )
    .await?;

    // Then: only the prompt_answer grant answers, and the record shows exactly that answer.
    let kinds: Vec<&str> = results.iter().map(error_kind).collect();
    assert_eq!(kinds.get(..2), Some(&["forbidden", "forbidden"][..]));
    let accepted = results.last().ok_or("answerer reply")?;
    assert_eq!(at(accepted, "/result/state"), "answered", "{accepted}");
    assert_eq!(at(accepted, "/result/version"), "3");
    assert_eq!(at(&read, "/result"), at(accepted, "/result"));
    drop(owner);
    lane.close(server).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_retried_answer_after_disconnect_returns_its_receipt_and_answers_once() -> TestResult {
    let lane = lane()?;
    let server = lane.socket.serve(1, Arc::clone(&lane.dispatch)).await?;
    let mut owner = lane.client(&lane.socket.bearer).await?;
    ask(&mut owner, ("open", "prompt.open"), open()).await?;
    let retried = op();
    let first = ask(
        &mut owner,
        ("first", "prompt.answer"),
        answer(&retried, &json!("1"), &json!({"choice": "no"})),
    )
    .await?;
    drop(owner);

    let mut again = lane.client(&lane.socket.bearer).await?;
    let replay = ask(
        &mut again,
        ("replay", "prompt.answer"),
        answer(&retried, &json!("1"), &json!({"choice": "no"})),
    )
    .await?;
    let fresh = ask(
        &mut again,
        ("fresh", "prompt.answer"),
        answer(&op(), &json!("1"), &json!({"choice": "no"})),
    )
    .await?;
    let listed = ask(
        &mut again,
        ("list", "prompt.list"),
        json!({"target": {"name": "agent"}}),
    )
    .await?;

    assert_eq!(
        at(&first, "/result/answer/op_id"),
        retried.as_str(),
        "{first}"
    );
    assert_eq!(at(&replay, "/result"), at(&first, "/result"));
    assert_eq!(error_kind(&fresh), "version_conflict", "{fresh}");
    assert_eq!(at(&fresh, "/error/data/current_version"), "2");
    assert_eq!(at(&listed, "/result/prompts/0/version"), "2", "{listed}");
    drop(again);
    lane.close(server).await
}

#[tokio::test(flavor = "multi_thread")]
async fn numeric_versions_and_untagged_answers_are_invalid_params_before_any_effect() -> TestResult
{
    let lane = lane()?;
    let server = lane.socket.serve(1, Arc::clone(&lane.dispatch)).await?;
    let mut owner = lane.client(&lane.socket.bearer).await?;
    ask(&mut owner, ("open", "prompt.open"), open()).await?;

    let numeric = ask(
        &mut owner,
        ("numeric", "prompt.answer"),
        answer(&op(), &json!(1), &json!({"choice": "yes"})),
    )
    .await?;
    let untagged = ask(
        &mut owner,
        ("untagged", "prompt.answer"),
        answer(&op(), &json!("1"), &json!("yes")),
    )
    .await?;
    let unknown = ask(
        &mut owner,
        ("unknown", "prompt.get"),
        json!({"prompt_id": "p9"}),
    )
    .await?;
    let read = ask(
        &mut owner,
        ("get", "prompt.get"),
        json!({"prompt_id": "p1"}),
    )
    .await?;

    assert_eq!(error_kind(&numeric), "invalid_params", "{numeric}");
    assert_eq!(at(&numeric, "/error/data/field_path"), "params.version");
    assert_eq!(error_kind(&untagged), "invalid_params", "{untagged}");
    assert_eq!(at(&untagged, "/error/data/field_path"), "params.answer");
    assert_eq!(error_kind(&unknown), "not_found", "{unknown}");
    assert_eq!(
        (at(&read, "/result/state"), at(&read, "/result/version")),
        (&json!("open"), &json!("1"))
    );
    drop(owner);
    lane.close(server).await
}
