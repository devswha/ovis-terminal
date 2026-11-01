//! The stand-in remote runtime at the far end of a fixture machine's line.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use ovis_terminal_agent::tree::{TreeError, TreeService};
use ovis_terminal_authz::PrincipalId;
use ovis_terminal_core::ops::{Completion, Context, Operations, Request};
use ovis_terminal_core::registry::RuntimeIdentity;
use ovis_terminal_protocol::domain::machine_actor::{ORPHANED, OrphanedParams};
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind, OpId};
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::records::ParentRef;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_remote::actors::Lines;
use ovis_terminal_remote::orphans::Inbox;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream, ReadHalf, WriteHalf,
};
use tokio::task::JoinHandle;

use super::Core;

pub(super) type PipeLine = Lines<ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>;

/// One remote machine; its applied notices outlive any single line to it.
#[derive(Debug)]
pub(super) struct Remote {
    id: MachineId,
    inbox: Inbox,
    applied: AtomicU64,
    cancels: AtomicU64,
    frozen: AtomicBool,
    line: Mutex<Option<JoinHandle<()>>>,
}

impl Remote {
    pub(super) fn new(id: MachineId) -> Arc<Self> {
        Arc::new(Self {
            inbox: Inbox::new(id.clone()),
            id,
            applied: AtomicU64::new(0),
            cancels: AtomicU64::new(0),
            frozen: AtomicBool::new(false),
            line: Mutex::new(None),
        })
    }

    pub(super) const fn id(&self) -> &MachineId {
        &self.id
    }

    fn replace(&self, task: Option<JoinHandle<()>>) {
        let mut line = self.line.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(previous) = std::mem::replace(&mut *line, task) {
            previous.abort();
        }
    }

    /// A fresh, answering line; the previous one is cut.
    pub(super) fn connect(self: &Arc<Self>) -> PipeLine {
        let (local, remote) = tokio::io::duplex(64 * 1024);
        let (local_read, local_write) = tokio::io::split(local);
        let (remote_read, remote_write) = tokio::io::split(remote);
        self.frozen.store(false, Ordering::SeqCst);
        let task = tokio::spawn(Arc::clone(self).serve(BufReader::new(remote_read), remote_write));
        self.replace(Some(task));
        Lines::new(local_read, local_write)
    }

    /// From now on requests are read and never answered.
    pub(super) fn freeze(&self) {
        self.frozen.store(true, Ordering::SeqCst);
    }

    /// Ends the line by dropping its remote end.
    pub(super) fn cut(&self) {
        self.replace(None);
    }

    pub(super) fn counters(&self) -> Value {
        json!({
            "applied": self.applied.load(Ordering::SeqCst).to_string(),
            "cancels": self.cancels.load(Ordering::SeqCst).to_string(),
            "frozen": self.frozen.load(Ordering::SeqCst),
        })
    }

    async fn serve(
        self: Arc<Self>,
        mut reader: BufReader<ReadHalf<DuplexStream>>,
        mut writer: WriteHalf<DuplexStream>,
    ) {
        let mut record = String::new();
        while matches!(reader.read_line(&mut record).await, Ok(1..)) {
            let message: Value = serde_json::from_str(&record).unwrap_or(Value::Null);
            record.clear();
            if message.get("id").is_none() {
                if message.get("method").and_then(Value::as_str) == Some("$/cancelRequest") {
                    self.cancels.fetch_add(1, Ordering::SeqCst);
                }
                continue;
            }
            if self.frozen.load(Ordering::SeqCst) {
                continue;
            }
            let reply = self.answer(&message).await;
            if writer
                .write_all(format!("{reply}\n").as_bytes())
                .await
                .is_err()
            {
                break;
            }
        }
    }

    async fn answer(&self, request: &Value) -> Value {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let refused = |kind: &str| {
            json!({"jsonrpc": "2.0", "id": id, "error": {
                "code": -32000, "message": kind, "data": {"kind": kind, "retryable": false},
            }})
        };
        match request.get("method").and_then(Value::as_str) {
            Some("server.status") => {
                json!({"jsonrpc": "2.0", "id": id, "result": {"machine_id": self.id}})
            }
            Some(ORPHANED) => {
                let Ok(params) = serde_json::from_value::<OrphanedParams>(
                    request.get("params").cloned().unwrap_or(Value::Null),
                ) else {
                    return refused("invalid_params");
                };
                let received = self.inbox.receive(&params, |_| async {
                    self.applied.fetch_add(1, Ordering::SeqCst);
                    Ok(true)
                });
                match received.await {
                    Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                    Err(error) => refused(error.kind()),
                }
            }
            _ => refused("method_not_found"),
        }
    }
}

/// A fixture failure that is no fault of the request.
pub(super) fn internal<E>(_: E) -> BoundaryError {
    BoundaryError::domain(ErrorKind::Internal, false).with_hint("inspect the fixture's stderr")
}

/// Commits, as one operation, that `machine` holds a child of the live `parent`.
pub(super) fn link(
    tree: &TreeService,
    (core, runtime): (&Core, RuntimeIdentity),
    (parent, machine): (ParentRef, MachineId),
) -> Result<Value, BoundaryError> {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(internal)?;
    let now = TimestampMs::new(u64::try_from(since.as_millis()).map_err(internal)?);
    let owner = PrincipalId::parse("owner").map_err(internal)?;
    let operations = Operations::new(core, Context::new(owner, runtime, now));
    let op = OpId::new(uuid::Uuid::now_v7().to_string());
    let request =
        Request::new(op, "fixture.remote_child".to_owned(), json!({})).map_err(internal)?;
    operations.accept(&request).map_err(internal)?;
    let permit = operations.start(request.op_id()).map_err(internal)?;
    let record = operations
        .commit_unit(permit, |snapshot| {
            TreeError::settle(tree.link_remote(snapshot, (parent, machine)).map(|writes| {
                writes
                    .into_iter()
                    .fold(Completion::new(json!({"linked": true})), Completion::write)
            }))
        })
        .map_err(internal)?;
    match record.error {
        Some(error) => Err(BoundaryError::Domain {
            data: Box::new(error),
        }),
        None => Ok(record.result.unwrap_or(Value::Null)),
    }
}
