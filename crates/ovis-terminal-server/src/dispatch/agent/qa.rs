//! The assembled agent domain over one dispatcher, gate and socket.

use std::collections::BTreeSet;
use std::sync::Arc;

use ovis_terminal_agent::conversation::binding::Stores;
use ovis_terminal_agent::status::{AgentStatusEvent, ProcessOwner};
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::OperationProducer;
use ovis_terminal_core::registry::{TargetResolutionError, TerminalGeneration, TerminalId};
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::ids::ProcessToken;
use ovis_terminal_protocol::schema::methods::all_methods;
use ovis_terminal_protocol::schema::scalar::Incarnation;
use ovis_terminal_server::conn::RequestHandler as _;
use ovis_terminal_server::dispatch::Dispatcher;
use ovis_terminal_server::dispatch::agent::{AGENT_METHODS, AgentAssembly};
use ovis_terminal_server::dispatch::lanes::agent_status::{ResolvedTerminal, TerminalDirectory};
use ovis_terminal_server::listen::ServerHandle;
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};

use super::support::socket::{self, Client};
use super::support::{TestResult, at, error_kind, request, runtime};

type Dispatch = Dispatcher<AgentStatusEvent, OperationProducer>;
type Core = Coordinator<AgentStatusEvent, OperationProducer>;

/// Terminal 4 named `agent` and terminal 5 named `other`, each at generation 1.
#[derive(Debug, Clone)]
struct Two(ResolvedTerminal, ResolvedTerminal);

impl TerminalDirectory for Two {
    fn resolve(&self, target: &Target) -> Result<ResolvedTerminal, TargetResolutionError> {
        match target {
            Target::Name(name) if name.as_str() == "agent" => Ok(self.0),
            Target::Name(name) if name.as_str() == "other" => Ok(self.1),
            Target::TerminalId(id) => [self.0, self.1]
                .into_iter()
                .find(|known| id.as_str() == known.terminal.to_string())
                .ok_or(TargetResolutionError::NotFound),
            _ => Err(TargetResolutionError::NotFound),
        }
    }
}

struct Rig {
    socket: socket::Fixture,
    core: Arc<Core>,
    dispatch: Dispatch,
    assembly: AgentAssembly<Two>,
}

fn rig() -> TestResult<Rig> {
    let socket = socket::Fixture::new()?;
    let identity = runtime(1)?;
    let resolved = |id: u64| -> TestResult<ResolvedTerminal> {
        let terminal = TerminalId::try_from(id)?;
        Ok(ResolvedTerminal::new(
            identity.machine_id(),
            terminal,
            TerminalGeneration::new(1)?,
        ))
    };
    let directory = Two(resolved(4)?, resolved(5)?);
    std::fs::create_dir(socket.path("store"))?;
    let stores = Stores::new(&[("store", socket.path("store"))])?;
    let assembly = AgentAssembly::new(identity, directory.clone(), (stores, None));
    for (terminal, token) in [(&directory.0, "token-agent"), (&directory.1, "token-other")] {
        let owner = ProcessOwner::new(terminal.terminal, Incarnation::new(1));
        assembly
            .status()
            .tokens()
            .bind(&ProcessToken::new(token.to_owned()), owner)?;
    }
    let store = Store::open(&socket.path("db"), StoreOptions::default())?;
    let producer = OperationProducer::new(identity.runtime_id());
    let core = Arc::new(Coordinator::new(store, identity, producer));
    let dispatch = Dispatch::new(Arc::clone(&socket.authority), identity, Arc::clone(&core));
    Ok(Rig {
        socket,
        core,
        dispatch,
        assembly,
    })
}

async fn ask(client: &mut Client, (id, method): (&str, &str), params: Value) -> TestResult<Value> {
    client.send(&request(id, method, &params)).await?;
    let reply = client.expect_next().await?;
    assert_eq!(at(&reply, "/id"), id, "{reply}");
    Ok(reply)
}

async fn connect(socket: &socket::Fixture) -> TestResult<Client> {
    let mut client = socket.connect().await?;
    client.hello("1", &socket.bearer).await?;
    Ok(client)
}

async fn close((core, dispatch): (Arc<Core>, Arc<Dispatch>), server: ServerHandle) -> TestResult {
    server.shutdown().await?;
    dispatch.shutdown().await?;
    drop(dispatch);
    let core = Arc::try_unwrap(core).map_err(|_| "core still owned")?;
    tokio::task::spawn_blocking(move || core.close()).await??;
    Ok(())
}

#[test]
fn the_assembly_routes_exactly_its_catalog_methods_once() -> TestResult {
    // Given an empty dispatcher and the assembly.
    let mut rig = rig()?;
    let before: BTreeSet<&str> = rig.dispatch.methods().into_iter().collect();
    // When it registers, and then registers again.
    rig.assembly.register(&mut rig.dispatch)?;
    let again = rig.assembly.register(&mut rig.dispatch);
    // Then exactly the declared catalog methods were added, and nothing is replaced.
    let after: BTreeSet<&str> = rig.dispatch.methods().into_iter().collect();
    let added: BTreeSet<&str> = after.difference(&before).copied().collect();
    assert_eq!(added, AGENT_METHODS.into_iter().collect::<BTreeSet<_>>());
    let catalog: BTreeSet<&str> = all_methods().map(|spec| spec.name).collect();
    assert!(added.is_subset(&catalog), "{added:?}");
    assert!(
        again.is_err(),
        "a second registration must not replace a route"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn two_clients_share_one_agent_record_behind_the_bridge_gate() -> TestResult {
    // Given the assembly served behind its gate, and two connections.
    let mut rig = rig()?;
    rig.assembly.register(&mut rig.dispatch)?;
    let Rig {
        socket,
        core,
        dispatch,
        assembly,
    } = rig;
    let dispatch = Arc::new(dispatch);
    let server = socket
        .serve(1, Arc::new(assembly.gate(Arc::clone(&dispatch))))
        .await?;
    let (mut first, mut second) = (connect(&socket).await?, connect(&socket).await?);
    // When the first registers an unknown brand and the second reads it.
    let register = json!({"op_id": op(), "target": {"name": "agent"},
        "process_token": "token-agent", "label": "smoke-agent"});
    let registered = ask(&mut first, ("register", "agent.register"), register.clone()).await?;
    let read = ask(
        &mut second,
        ("get", "agent.get"),
        json!({"target": {"name": "agent"}}),
    )
    .await?;
    // Then both see the same declared record, with no brand list consulted.
    assert_eq!(
        at(&registered, "/result/agent/label"),
        "smoke-agent",
        "{registered}"
    );
    assert_eq!(at(&read, "/result/agent"), at(&registered, "/result/agent"));
    // And a stale report is a typed refusal naming the current sequence.
    let report = |seq: &str| {
        json!({"op_id": op(), "target": {"name": "agent"},
        "process_token": "token-agent", "seq": seq, "record": {"state": "working"}})
    };
    let applied = ask(&mut first, ("report-2", "status.report"), report("2")).await?;
    assert_eq!(at(&applied, "/result/status/report_seq"), "2", "{applied}");
    let stale = ask(&mut second, ("report-1", "status.report"), report("1")).await?;
    assert_eq!(
        (error_kind(&stale), at(&stale, "/error/data/current_seq")),
        ("stale_seq", &json!("2"))
    );
    // And another program's token cannot open a prompt on this terminal.
    let foreign = json!({"op_id": op(), "target": {"name": "agent"}, "process_token": "token-other",
        "prompt_id": "p1", "prompt": {"kind": "permission", "text": "Proceed?", "mechanism": "rpc",
        "choices": [{"id": "yes", "label": "Yes"}]}});
    let refused = ask(&mut first, ("foreign", "prompt.open"), foreign).await?;
    assert_eq!(error_kind(&refused), "forbidden", "{refused}");
    let forced = json!({"op_id": op(), "prompt_id": "p1", "version": "1",
        "answer": {"choice": "yes"}, "force": true});
    let forced = ask(&mut second, ("forced", "prompt.answer"), forced).await?;
    assert_eq!(error_kind(&forced), "invalid_params", "{forced}");
    let listed = ask(
        &mut second,
        ("list", "prompt.list"),
        json!({"target": {"name": "agent"}}),
    )
    .await?;
    assert_eq!(at(&listed, "/result/prompts"), &json!([]), "{listed}");
    // And after a disconnect the retried registration returns its retained receipt.
    drop(first);
    let mut third = connect(&socket).await?;
    let replay = ask(&mut third, ("replay", "agent.register"), register).await?;
    assert_eq!(at(&replay, "/result"), at(&registered, "/result"));
    drop((second, third, assembly));
    close((core, dispatch), server).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_retry_of_a_committed_answer_still_inspects_as_committed() -> TestResult {
    // Given p1 answered once under operation X.
    let mut rig = rig()?;
    rig.assembly.register(&mut rig.dispatch)?;
    let Rig {
        socket,
        core,
        dispatch,
        assembly,
    } = rig;
    let dispatch = Arc::new(dispatch);
    let server = socket
        .serve(1, Arc::new(assembly.gate(Arc::clone(&dispatch))))
        .await?;
    let (mut first, mut second) = (connect(&socket).await?, connect(&socket).await?);
    let open = json!({"op_id": op(), "target": {"name": "agent"}, "process_token": "token-agent",
        "prompt_id": "p1", "prompt": {"kind": "permission", "text": "Proceed?", "mechanism": "rpc",
        "choices": [{"id": "yes", "label": "Yes"}]}});
    ask(&mut first, ("open", "prompt.open"), open).await?;
    let answer = json!({"op_id": op(), "prompt_id": "p1", "version": "1",
        "answer": {"choice": "yes"}});
    let answered = ask(&mut first, ("answer", "prompt.answer"), answer.clone()).await?;
    assert_eq!(at(&answered, "/result/state"), "answered", "{answered}");
    // When every pinned-snapshot slot is held, so X's retry provably waits in the queue.
    let mut leases = Vec::new();
    while let Ok(lease) = core.pin_snapshot() {
        leases.push(lease);
    }
    let retry = ask(&mut first, ("retry", "prompt.answer"), answer.clone()).await?;
    assert_eq!(error_kind(&retry), "deadline_exceeded", "{retry}");
    assert!(
        at(&retry, "/error/data/queued_position").is_string(),
        "{retry}"
    );
    let inspected = ask(
        &mut second,
        ("get", "op.get"),
        json!({"op_id": at(&answer, "/op_id")}),
    )
    .await?;
    // Then inspection reports the durable outcome, not the retry's queue place.
    assert_eq!(at(&inspected, "/result/state"), "committed", "{inspected}");
    assert_eq!(
        at(&inspected, "/result/queued_position"),
        &Value::Null,
        "{inspected}"
    );
    for lease in leases {
        core.release_snapshot(&lease.token)?;
    }
    dispatch.drain().await?;
    drop((first, second, assembly));
    close((core, dispatch), server).await
}

fn op() -> String {
    uuid::Uuid::now_v7().to_string()
}
