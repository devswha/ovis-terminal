//! The session binding lane over a real dispatcher, store and socket.
//!
//! Replaced programs, replayed branch selections and malformed or stale cursors are
//! checked on the wire, where the operation ledger and parameter decoding take part.

use std::sync::Arc;

use ovis_terminal_agent::conversation::binding::{BindingService, Stores};
use ovis_terminal_agent::conversation::records::RecordsService;
use ovis_terminal_agent::status::{AgentStatusEvent, ProcessOwner, StatusService};
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::OperationProducer;
use ovis_terminal_core::registry::{TargetResolutionError, TerminalGeneration, TerminalId};
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::ids::ProcessToken;
use ovis_terminal_protocol::schema::scalar::Incarnation;
use ovis_terminal_server::dispatch::Dispatcher;
use ovis_terminal_server::dispatch::lanes::agent_status::{
    AgentStatusLane, RegisterAgent, ResolvedTerminal, TerminalDirectory,
};
use ovis_terminal_server::dispatch::lanes::conversation_records::{
    Conversation, ConversationRecordsLane,
};
use ovis_terminal_server::dispatch::lanes::session_binding::{SessionBindingLane, UpdateSession};
use ovis_terminal_server::listen::ServerHandle;
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};

use super::support::socket::{self, Client};
use super::support::{TestResult, at, error_kind, request, runtime};

type Dispatch = Dispatcher<AgentStatusEvent, OperationProducer>;
type Core = Coordinator<AgentStatusEvent, OperationProducer>;

const FIRST: &str = "token-first";
const SECOND: &str = "token-second";

/// Terminal 4, named `agent`, at generation 1; every other target is unknown.
#[derive(Debug)]
struct OneTerminal(ResolvedTerminal);

impl TerminalDirectory for OneTerminal {
    fn resolve(&self, target: &Target) -> Result<ResolvedTerminal, TargetResolutionError> {
        match target {
            Target::Name(name) if name.as_str() == "agent" => Ok(self.0),
            _ => Err(TargetResolutionError::NotFound),
        }
    }
}

struct Lane {
    socket: socket::Fixture,
    status: Arc<StatusService>,
    core: Arc<Core>,
    dispatch: Arc<Dispatch>,
}

fn lane() -> TestResult<Lane> {
    let socket = socket::Fixture::new()?;
    let identity = runtime(1)?;
    let store_dir = socket.path("store");
    std::fs::create_dir(&store_dir)?;
    let terminal = TerminalId::try_from(4)?;
    let status = Arc::new(StatusService::new(identity.runtime_id()));
    status.tokens().bind(
        &ProcessToken::new(FIRST.to_owned()),
        ProcessOwner::new(terminal, Incarnation::new(1)),
    )?;
    let directory = Arc::new(OneTerminal(ResolvedTerminal::new(
        identity.machine_id(),
        terminal,
        TerminalGeneration::new(1)?,
    )));
    let store = Store::open(&socket.path("db"), StoreOptions::default())?;
    let core = Arc::new(Coordinator::new(
        store,
        identity,
        OperationProducer::new(identity.runtime_id()),
    ));
    let mut dispatch = Dispatch::new(Arc::clone(&socket.authority), identity, Arc::clone(&core));
    let agents = AgentStatusLane::new(Arc::clone(&directory), Arc::clone(&status));
    dispatch.register(RegisterAgent::new(&agents))?;
    let stores = Stores::new(&[("store", store_dir)])?;
    let service = Arc::new(BindingService::new(
        identity.runtime_id(),
        Arc::clone(&status),
        stores,
    ));
    let records = Arc::new(RecordsService::new(
        identity.runtime_id(),
        Arc::clone(&status),
    ));
    let binding = SessionBindingLane::new(Arc::clone(&directory), Arc::clone(&service));
    dispatch.register(UpdateSession::new(&binding))?;
    let conversation = ConversationRecordsLane::new(directory, service, records);
    dispatch.register(Conversation::new(&conversation))?;
    Ok(Lane {
        socket,
        status,
        core,
        dispatch: Arc::new(dispatch),
    })
}

impl Lane {
    async fn connect(&self) -> TestResult<(ServerHandle, Client)> {
        let server = self.socket.serve(1, Arc::clone(&self.dispatch)).await?;
        let mut client = self.socket.connect().await?;
        client.hello("1", &self.socket.bearer).await?;
        Ok((server, client))
    }

    async fn close(self, server: ServerHandle) -> TestResult {
        server.shutdown().await?;
        self.dispatch.shutdown().await?;
        drop(self.dispatch);
        let core = Arc::try_unwrap(self.core).map_err(|_| "core still owned")?;
        tokio::task::spawn_blocking(move || core.close()).await??;
        Ok(())
    }

    fn transcript(&self, text: &str) -> TestResult {
        Ok(std::fs::write(
            self.socket.path("store").join("one.jsonl"),
            text,
        )?)
    }
}

async fn ask(client: &mut Client, (id, method): (&str, &str), params: Value) -> TestResult<Value> {
    client.send(&request(id, method, &params)).await?;
    let reply = client.expect_next().await?;
    assert_eq!(at(&reply, "/id"), id, "{reply}");
    Ok(reply)
}

fn update(op: &str, token: &str, extra: &Value) -> Value {
    let mut params = json!({"op_id": op, "target": {"name": "agent"}, "process_token": token,
        "session": {"source": "store", "kind": "path", "value": "one.jsonl"}, "history_id": "h1"});
    if let (Some(params), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
        params.extend(extra.clone());
    }
    params
}

fn op() -> String {
    uuid::Uuid::now_v7().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_replaced_programs_token_is_refused_and_a_replayed_branch_applies_once() -> TestResult {
    // Given: incarnation 1 registered and bound to h1 = r1 <- r2.
    let lane = lane()?;
    lane.transcript("{\"id\":\"r1\",\"role\":\"user\"}\n{\"id\":\"r2\",\"role\":\"user\"}\n")?;
    let (server, mut client) = lane.connect().await?;
    let register = json!({"op_id": op(), "target": {"name": "agent"},
        "process_token": FIRST, "label": "custom-agent"});
    ask(&mut client, ("register", "agent.register"), register).await?;
    let bound = ask(
        &mut client,
        ("bind", "agent.update_session"),
        update(&op(), FIRST, &json!({})),
    )
    .await?;
    assert_eq!(at(&bound, "/result/agent/history_id"), "h1", "{bound}");

    // When: the program is replaced; the second one registers and selects r1 twice
    // under one op_id, as a retry after an interrupted reply would.
    lane.status.tokens().bind(
        &ProcessToken::new(SECOND.to_owned()),
        ProcessOwner::new(TerminalId::try_from(4)?, Incarnation::new(2)),
    )?;
    let stale = ask(
        &mut client,
        ("stale", "agent.update_session"),
        update(&op(), FIRST, &json!({"reason": "resume"})),
    )
    .await?;
    let register = json!({"op_id": op(), "target": {"name": "agent"},
        "process_token": SECOND, "label": "custom-agent"});
    ask(&mut client, ("register-2", "agent.register"), register).await?;
    let rebind = ask(
        &mut client,
        ("rebind", "agent.update_session"),
        update(&op(), SECOND, &json!({})),
    )
    .await?;
    let branch = update(&op(), SECOND, &json!({"reason": "branch", "head": "r1"}));
    let once = ask(
        &mut client,
        ("branch-1", "agent.update_session"),
        branch.clone(),
    )
    .await?;
    let again = ask(&mut client, ("branch-2", "agent.update_session"), branch).await?;
    let page = ask(
        &mut client,
        ("page", "agent.conversation"),
        json!({"target": {"name": "agent"}}),
    )
    .await?;

    // Then: the old token is not an owner, the replay returns the first outcome, and the
    // history moved exactly one revision for the selection.
    assert_eq!(error_kind(&stale), "not_owner", "{stale}");
    assert_eq!(
        at(&rebind, "/result/agent/process_incarnation"),
        "2",
        "{rebind}"
    );
    assert_eq!(at(&once, "/result"), at(&again, "/result"));
    assert_eq!(at(&page, "/result/revision"), "2", "{page}");
    assert_eq!(at(&page, "/result/records/0/record_id"), "r1");
    assert_eq!(at(&page, "/result/off_branch"), &json!(["r2"]));
    drop(client);
    lane.close(server).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_numeric_revision_is_invalid_params_and_a_stale_one_names_the_current_revision()
-> TestResult {
    let lane = lane()?;
    lane.transcript("{\"id\":\"r1\",\"role\":\"user\"}\n")?;
    let (server, mut client) = lane.connect().await?;
    let register = json!({"op_id": op(), "target": {"name": "agent"},
        "process_token": FIRST, "label": "custom-agent"});
    ask(&mut client, ("register", "agent.register"), register).await?;
    ask(
        &mut client,
        ("bind", "agent.update_session"),
        update(&op(), FIRST, &json!({})),
    )
    .await?;
    lane.transcript("{\"id\":\"q1\",\"role\":\"user\"}\n")?;
    let cursor = |revision: Value| {
        json!({"target": {"name": "agent"},
               "cursor": {"history_id": "h1", "revision": revision, "position": "0"}})
    };

    let numeric = ask(
        &mut client,
        ("numeric", "agent.conversation"),
        cursor(json!(1)),
    )
    .await?;
    let stale = ask(
        &mut client,
        ("stale", "agent.conversation"),
        cursor(json!("1")),
    )
    .await?;

    assert_eq!(error_kind(&numeric), "invalid_params", "{numeric}");
    assert_eq!(
        at(&numeric, "/error/data/field_path"),
        "params.cursor.revision"
    );
    assert_eq!(error_kind(&stale), "history_changed", "{stale}");
    assert_eq!(
        at(&stale, "/error/data/history"),
        &json!({"history_id": "h1", "revision": "2", "change": "rewrite"})
    );
    assert_eq!(at(&stale, "/result"), &Value::Null);
    drop(client);
    lane.close(server).await
}
