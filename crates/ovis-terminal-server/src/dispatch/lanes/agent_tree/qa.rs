//! `agent.children` over a real dispatcher, authority, store and socket.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_agent::prompts::PromptService;
use ovis_terminal_agent::status::{AgentStatusEvent, ProcessOwner, StatusService};
use ovis_terminal_agent::tree::{Link, TreeError, TreeService};
use ovis_terminal_authz::PrincipalId;
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::{Completion, Context, OperationProducer, Operations, Request};
use ovis_terminal_core::registry::{TargetResolutionError, TerminalGeneration, TerminalId};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::ids::{ProcessToken, TerminalId as WireTerminal};
use ovis_terminal_protocol::schema::methods::terminal::ChildPolicy;
use ovis_terminal_protocol::schema::records::ParentRef;
use ovis_terminal_protocol::schema::scalar::{Incarnation, TimestampMs};
use ovis_terminal_server::dispatch::Dispatcher;
use ovis_terminal_server::dispatch::lanes::agent_status::{ResolvedTerminal, TerminalDirectory};
use ovis_terminal_server::dispatch::lanes::agent_tree::{AgentTreeLane, ListChildren};
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};

use super::support::socket::{self, Client};
use super::support::{TestResult, at, error_kind, request, runtime};

type Dispatch = Dispatcher<AgentStatusEvent, OperationProducer>;
type Core = Coordinator<AgentStatusEvent, OperationProducer>;

/// Terminal 6 named `parent`; every other name is unknown.
#[derive(Debug)]
struct Parent(ResolvedTerminal);

impl TerminalDirectory for Parent {
    fn resolve(&self, target: &Target) -> Result<ResolvedTerminal, TargetResolutionError> {
        match target {
            Target::Name(name) if name.as_str() == "parent" => Ok(self.0),
            _ => Err(TargetResolutionError::NotFound),
        }
    }
}

fn bind(status: &StatusService, (terminal, incarnation): (u64, u64)) -> TestResult<ProcessOwner> {
    let owner = ProcessOwner::new(
        TerminalId::try_from(terminal)?,
        Incarnation::new(incarnation),
    );
    let token = ProcessToken::new(format!("token-{terminal}-{incarnation}"));
    status.tokens().bind(&token, owner)?;
    Ok(owner)
}

/// Links `child` to parent incarnation 1 of terminal 6 in one operation unit.
fn link(core: &Core, tree: &TreeService, child: ProcessOwner) -> TestResult {
    let op = OpId::new(uuid::Uuid::now_v7().to_string());
    let request = Request::new(op, "tree.seed".to_owned(), json!({}))?;
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let now = TimestampMs::new(u64::try_from(elapsed)?);
    let operations = Operations::new(
        core,
        Context::new(PrincipalId::parse("owner")?, runtime(1)?, now),
    );
    operations.accept(&request)?;
    let permit = operations.start(request.op_id())?;
    let parent = ParentRef::new(
        WireTerminal::new("terminal:6".to_owned()),
        Incarnation::new(1),
    );
    let record = operations.commit_unit(permit, |snapshot| {
        TreeError::settle((|| {
            let link = Link::new(parent, child, ChildPolicy::new(true, true));
            let linked = tree.link(snapshot, link)?;
            Ok(linked
                .writes
                .into_iter()
                .fold(Completion::new(json!({})), Completion::write))
        })())
    })?;
    assert_eq!(record.error, None);
    Ok(())
}

async fn ask(client: &mut Client, id: &str, params: Value) -> TestResult<Value> {
    client.send(&request(id, "agent.children", &params)).await?;
    let reply = client.expect_next().await?;
    assert_eq!(at(&reply, "/id"), id, "{reply}");
    Ok(reply)
}

#[tokio::test(flavor = "multi_thread")]
async fn children_belong_to_the_current_incarnation_and_counters_are_decimal() -> TestResult {
    let socket = socket::Fixture::new()?;
    let identity = runtime(1)?;
    let store = Store::open(&socket.path("db"), StoreOptions::default())?;
    let producer = OperationProducer::new(identity.runtime_id());
    let core = Arc::new(Coordinator::new(store, identity, producer));
    let status = Arc::new(StatusService::new(identity.runtime_id()));
    let prompts = Arc::new(PromptService::new(
        identity.runtime_id(),
        Arc::clone(&status),
    ));
    let tree = Arc::new(TreeService::new(identity, (Arc::clone(&status), prompts)));
    bind(&status, (6, 1))?;
    let child = bind(&status, (7, 1))?;
    tokio::task::block_in_place(|| link(&core, &tree, child))?;
    let resolved = ResolvedTerminal::new(
        identity.machine_id(),
        TerminalId::try_from(6)?,
        TerminalGeneration::new(1)?,
    );
    let mut dispatch = Dispatch::new(Arc::clone(&socket.authority), identity, Arc::clone(&core));
    dispatch.register_read(ListChildren::new(&AgentTreeLane::new(
        Parent(resolved),
        Arc::clone(&tree),
    )))?;
    let dispatch = Arc::new(dispatch);
    let server = socket.serve(1, Arc::clone(&dispatch)).await?;
    let mut client = socket.connect().await?;
    client.hello("1", &socket.bearer).await?;

    let listed = ask(&mut client, "list", json!({"target": {"name": "parent"}})).await?;
    assert_eq!(
        at(&listed, "/result/children/0/terminal_id"),
        "terminal:7",
        "{listed}"
    );
    assert_eq!(at(&listed, "/result/children/0/process_incarnation"), "1");
    assert!(at(&listed, "/result/revision").is_string(), "{listed}");
    let unknown = ask(
        &mut client,
        "unknown",
        json!({"target": {"name": "nobody"}}),
    )
    .await?;
    assert_eq!(error_kind(&unknown), "not_found");
    let extra = json!({"target": {"name": "parent"}, "include_finished": true});
    assert_eq!(
        error_kind(&ask(&mut client, "extra", extra).await?),
        "invalid_params"
    );

    bind(&status, (6, 2))?;
    let reused = ask(&mut client, "reused", json!({"target": {"name": "parent"}})).await?;
    assert_eq!(at(&reused, "/result/children"), &json!([]), "{reused}");

    drop(client);
    server.shutdown().await?;
    dispatch.shutdown().await?;
    drop(dispatch);
    drop(tree);
    let core = Arc::try_unwrap(core).map_err(|_| "core still owned")?;
    tokio::task::spawn_blocking(move || core.close()).await??;
    Ok(())
}
