//! The documents lane served over a private socket by the real dispatcher, ledger and store.

use std::sync::Arc;

use ovis_terminal_core::docs::DocumentsRetentionEvent;
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::OperationProducer;
use ovis_terminal_server::dispatch::Dispatcher;
use ovis_terminal_server::dispatch::lanes::documents_retention;
use ovis_terminal_server::listen::ServerHandle;
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};

use crate::socket::{self, Client, at, request, runtime};

pub(crate) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
pub(crate) type Core = Coordinator<DocumentsRetentionEvent, OperationProducer>;

pub(crate) struct Served {
    pub(crate) socket: socket::Fixture,
    pub(crate) core: Arc<Core>,
    dispatch: Arc<Dispatcher<DocumentsRetentionEvent, OperationProducer>>,
    server: ServerHandle,
}

impl Served {
    pub(crate) async fn start() -> TestResult<Self> {
        let socket = socket::Fixture::new()?;
        let identity = runtime(1)?;
        let store = Store::open(&socket.path("db"), StoreOptions::default())?;
        let producer = OperationProducer::new(identity.runtime_id());
        let core = Arc::new(Coordinator::new(store, identity, producer));
        let mut dispatch =
            Dispatcher::new(Arc::clone(&socket.authority), identity, Arc::clone(&core));
        documents_retention::register(&mut dispatch, identity)?;
        let dispatch = Arc::new(dispatch);
        let server = socket.serve(1, Arc::clone(&dispatch)).await?;
        Ok(Self {
            socket,
            core,
            dispatch,
            server,
        })
    }

    pub(crate) async fn client(&self) -> TestResult<Client> {
        let mut client = self.socket.connect().await?;
        client.hello("1", &self.socket.bearer).await?;
        Ok(client)
    }

    pub(crate) async fn close(self) -> TestResult {
        self.server.shutdown().await?;
        self.dispatch.shutdown().await?;
        drop(self.dispatch);
        let core = Arc::try_unwrap(self.core).map_err(|_| "core still shared")?;
        tokio::task::spawn_blocking(move || core.close()).await??;
        Ok(())
    }
}

/// A `UUIDv7` minted `age_ms` before the wall clock.
pub(crate) fn op(age_ms: u64) -> TestResult<String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?;
    let at = u64::try_from(now.as_millis())?
        .checked_sub(age_ms)
        .ok_or("before epoch")?;
    let stamp = uuid::Timestamp::from_unix(
        uuid::NoContext,
        at / 1000,
        u32::try_from(at % 1000)?
            .checked_mul(1_000_000)
            .ok_or("nanos")?,
    );
    Ok(uuid::Uuid::new_v7(stamp).to_string())
}

pub(crate) fn set(op_id: &str, mut params: Value) -> Value {
    if let Some(object) = params.as_object_mut() {
        object.insert("op_id".to_owned(), json!(op_id));
    }
    request(op_id, "doc.set", &params)
}

pub(crate) async fn call(client: &mut Client, message: &Value) -> TestResult<Value> {
    client.send(message).await?;
    client.expect_next().await
}

pub(crate) async fn get(client: &mut Client, name: &str) -> TestResult<Value> {
    let reply = call(client, &request("get", "doc.get", &json!({"name": name}))).await?;
    Ok(at(&reply, "/result").clone())
}
