//! The conversation record lane over a real dispatcher, store and socket.
//!
//! Parameter decoding, the operation ledger and source routing take part here: a screen
//! source and numeric counters never reach a service, and a replayed append commits once.

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
    AgentStatusLane, RegisterAgent, ReportStatus, ResolvedTerminal, TerminalDirectory,
};
use ovis_terminal_server::dispatch::lanes::conversation_records::{
    AppendRecords, Conversation, ConversationRecordsLane,
};
use ovis_terminal_server::listen::ServerHandle;
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};

use super::support::socket::{self, Client};
use super::support::{TestResult, at, error_kind, request, runtime};

type Dispatch = Dispatcher<AgentStatusEvent, OperationProducer>;
type Core = Coordinator<AgentStatusEvent, OperationProducer>;

const TOKEN: &str = "token-agent";

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
        &ProcessToken::new(TOKEN.to_owned()),
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
    dispatch.register(ReportStatus::new(&agents))?;
    let stores = Stores::new(&[("store", store_dir)])?;
    let binding = BindingService::new(identity.runtime_id(), Arc::clone(&status), stores);
    let records = RecordsService::new(identity.runtime_id(), status);
    let lane = ConversationRecordsLane::new(directory, Arc::new(binding), Arc::new(records));
    dispatch.register(AppendRecords::new(&lane))?;
    dispatch.register(Conversation::new(&lane))?;
    Ok(Lane {
        socket,
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

fn append(op: &str, records: &Value) -> Value {
    json!({"op_id": op, "target": {"name": "agent"}, "process_token": TOKEN, "records": records})
}

async fn registered(client: &mut Client) -> TestResult {
    let register = json!({"op_id": op(), "target": {"name": "agent"},
        "process_token": TOKEN, "label": "custom-agent"});
    let reply = ask(client, ("register", "agent.register"), register).await?;
    assert_eq!(at(&reply, "/error"), &Value::Null, "{reply}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_screen_source_and_numeric_counters_are_invalid_params_at_their_field() -> TestResult {
    let lane = lane()?;
    let (server, mut client) = lane.connect().await?;
    registered(&mut client).await?;

    let screen = ask(
        &mut client,
        ("screen", "agent.conversation"),
        json!({"target": {"name": "agent"}, "source": "terminal_screen"}),
    )
    .await?;
    let numeric = append(
        &op(),
        &json!([{"record_id": "u1", "role": "user", "at_ms": 7,
            "blocks": [{"type": "text", "text": "hi"}]}]),
    );
    let numeric = ask(&mut client, ("numeric", "agent.append_records"), numeric).await?;
    let unread = ask(
        &mut client,
        ("unread", "agent.conversation"),
        json!({"target": {"name": "agent"}, "source": "records"}),
    )
    .await?;

    assert_eq!(error_kind(&screen), "invalid_params", "{screen}");
    assert_eq!(at(&screen, "/error/data/field_path"), "params.source");
    assert_eq!(error_kind(&numeric), "invalid_params", "{numeric}");
    assert_eq!(
        at(&numeric, "/error/data/field_path"),
        "params.records[0].at_ms"
    );
    assert_eq!(error_kind(&unread), "conversation_unavailable", "{unread}");
    assert_eq!(
        at(&unread, "/error/data/unavailable/reason"),
        "not_reported"
    );
    drop(client);
    lane.close(server).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_replayed_append_commits_once_and_each_source_reads_its_own_records() -> TestResult {
    // Given: a registered agent in a running turn, with no transcript bound.
    let lane = lane()?;
    let (server, mut client) = lane.connect().await?;
    registered(&mut client).await?;
    let working = json!({"op_id": op(), "target": {"name": "agent"}, "process_token": TOKEN,
        "seq": "1", "record": {"state": "working"}});
    ask(&mut client, ("working", "status.report"), working).await?;

    // When: one append is sent twice under one op_id, then both sources are read.
    let once = append(
        &op(),
        &json!([{"record_id": "a1", "role": "assistant", "phase": "commentary",
            "blocks": [{"type": "tool_call", "call_id": "c1", "name": "ls", "input": {}}]},
            {"record_id": "t1", "role": "tool",
            "blocks": [{"type": "tool_result", "call_id": "c1", "output": "a b", "is_error": false}]}]),
    );
    let first = ask(&mut client, ("first", "agent.append_records"), once.clone()).await?;
    let again = ask(&mut client, ("again", "agent.append_records"), once).await?;
    let records = ask(
        &mut client,
        ("records", "agent.conversation"),
        json!({"target": {"name": "agent"}, "source": "records"}),
    )
    .await?;
    let transcript = ask(
        &mut client,
        ("transcript", "agent.conversation"),
        json!({"target": {"name": "agent"}}),
    )
    .await?;

    // Then: the replay returns the first outcome and the log holds the two records once.
    assert_eq!(at(&first, "/result"), at(&again, "/result"));
    assert_eq!(at(&records, "/result/revision"), "1", "{records}");
    assert_eq!(at(&records, "/result/page/total"), 2);
    assert_eq!(at(&records, "/result/records/1/links/0/call_record"), "a1");
    assert_eq!(at(&records, "/result/records/0/turn"), "1");
    assert_eq!(at(&records, "/result/turns/0/state"), "open");
    assert_eq!(
        at(&records, "/result/records"),
        at(&first, "/result/records")
    );
    assert_eq!(error_kind(&transcript), "conversation_unavailable");
    drop(client);
    lane.close(server).await
}
