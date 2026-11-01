//! The task that owns one machine's line, its calls in flight and their deadlines.

use std::collections::BTreeMap;
use std::time::Duration;

use ovis_terminal_protocol::domain::machine_actor::{EVENT, RemoteEvent};
use ovis_terminal_protocol::schema::ids::MachineId;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

mod expiry;

use super::ActorError;
use super::health::Health;
use super::line::Line;

/// Bound on writing a control record such as a cancellation or a refusal.
const CONTROL_WRITE: Duration = Duration::from_secs(1);

/// A remote's answer: its `result`, or its JSON-RPC `error` object unchanged.
pub type Reply = Result<Value, Value>;

/// What a machine's line delivered that no call waits for.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Inbound {
    /// A new line was attached.
    Attached(MachineId),
    /// The line was lost.
    Lost(MachineId),
    /// A notification from the remote, such as `event` or `heartbeat`.
    Notification {
        /// The machine.
        machine: MachineId,
        /// Notification method.
        method: String,
        /// Notification params.
        params: Value,
    },
}

pub(super) struct Call {
    pub method: String,
    pub params: Value,
    pub deadline: Duration,
    pub reply: oneshot::Sender<Result<Reply, ActorError>>,
}

pub(super) enum Command<L> {
    Call(Call),
    Attach(L),
}

struct Pending {
    reply: oneshot::Sender<Result<Reply, ActorError>>,
    deadline: Duration,
    due: Instant,
}

pub(super) struct Actor<L> {
    pub machine: MachineId,
    pub line: Option<L>,
    pub mailbox: mpsc::Receiver<Command<L>>,
    pub health: Health,
    pub inbound: mpsc::Sender<Inbound>,
    pending: BTreeMap<u64, Pending>,
    next: u64,
}

async fn receive<L: Line>(line: &mut Option<L>) -> Result<Vec<u8>, &'static str> {
    match line {
        Some(line) => line.receive().await.map_err(|error| error.kind()),
        None => std::future::pending().await,
    }
}

async fn until(due: Option<Instant>) {
    match due {
        Some(due) => tokio::time::sleep_until(due).await,
        None => std::future::pending().await,
    }
}

impl<L: Line> Actor<L> {
    pub(super) fn new(
        (machine, health): (MachineId, Health),
        mailbox: mpsc::Receiver<Command<L>>,
        inbound: mpsc::Sender<Inbound>,
    ) -> Self {
        Self {
            machine,
            line: None,
            mailbox,
            health,
            inbound,
            pending: BTreeMap::new(),
            next: 0,
        }
    }

    /// Serves until every handle to the mailbox is gone.
    pub(super) async fn run(mut self) {
        loop {
            if self
                .pending
                .values()
                .any(|pending| pending.reply.is_closed())
            {
                self.expire().await;
            }
            let due = self.pending.values().map(|pending| pending.due).min();
            tokio::select! {
                biased;
                command = self.mailbox.recv() => match command {
                    Some(Command::Call(call)) => self.call(call).await,
                    Some(Command::Attach(line)) => self.attach(line),
                    None => break,
                },
                received = receive(&mut self.line) => match received {
                    Ok(record) => self.received(&record).await,
                    Err(kind) => self.lose(kind),
                },
                () = until(due) => self.expire().await,
            }
        }
        self.lose("unavailable");
    }

    fn unreachable(&self) -> ActorError {
        ActorError::Unreachable {
            machine: self.machine.clone(),
            last_seen_seq: self.health.get().last_seen_seq,
        }
    }

    async fn call(&mut self, call: Call) {
        let Some(due) = Instant::now().checked_add(call.deadline) else {
            let requested = call.deadline;
            let refused = ActorError::DeadlineOutOfRange {
                requested,
                max: requested,
            };
            drop(call.reply.send(Err(refused)));
            return;
        };
        let id = self.next;
        self.next = self.next.saturating_add(1);
        let request = json!({
            "jsonrpc": "2.0", "id": id.to_string(), "method": call.method, "params": call.params,
        });
        let Some(line) = self.line.as_mut() else {
            drop(call.reply.send(Err(self.unreachable())));
            return;
        };
        let mut bytes = request.to_string().into_bytes();
        bytes.push(b'\n');
        match tokio::time::timeout_at(due, line.send(&bytes)).await {
            Ok(Ok(())) => {
                let pending = Pending {
                    reply: call.reply,
                    deadline: call.deadline,
                    due,
                };
                self.pending.insert(id, pending);
                self.health.in_flight(self.pending.len());
            }
            Ok(Err(error)) => {
                self.lose(error.kind());
                drop(call.reply.send(Err(self.unreachable())));
            }
            Err(_) => {
                self.lose("deadline_exceeded");
                let machine = self.machine.clone();
                let deadline = call.deadline;
                drop(
                    call.reply
                        .send(Err(ActorError::DeadlineExceeded { machine, deadline })),
                );
            }
        }
    }

    fn attach(&mut self, line: L) {
        if self.line.is_some() {
            self.lose("transport_failed");
        }
        self.line = Some(line);
        self.health.attached();
        self.forward(Inbound::Attached(self.machine.clone()));
    }

    /// Drops the line and fails every call in flight; the machine stays listed.
    fn lose(&mut self, kind: &'static str) {
        let had_line = self.line.take().is_some();
        let failed = std::mem::take(&mut self.pending);
        for pending in failed.into_values() {
            drop(pending.reply.send(Err(self.unreachable())));
        }
        if had_line {
            self.health.lost(kind);
            self.forward(Inbound::Lost(self.machine.clone()));
        }
    }

    fn forward(&self, inbound: Inbound) {
        if self.inbound.try_send(inbound).is_err() {
            self.health.dropped();
        }
    }

    async fn write_control(&mut self, record: &Value) {
        let mut bytes = record.to_string().into_bytes();
        bytes.push(b'\n');
        let Some(line) = self.line.as_mut() else {
            return;
        };
        match tokio::time::timeout(CONTROL_WRITE, line.send(&bytes)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => self.lose(error.kind()),
            Err(_) => self.lose("deadline_exceeded"),
        }
    }

    async fn received(&mut self, record: &[u8]) {
        let Ok(message) = serde_json::from_slice::<Value>(record) else {
            self.lose("transport_failed");
            return;
        };
        let method = message.get("method").and_then(Value::as_str);
        match (method, message.get("id")) {
            (None, Some(id)) => self.answered(id, &message),
            (Some(_), Some(id)) => {
                let refusal = json!({"jsonrpc": "2.0", "id": id, "error": {
                    "code": -32601, "message": "machine lines carry no requests to this runtime",
                    "data": {"kind": "method_not_found", "retryable": false},
                }});
                self.write_control(&refusal).await;
            }
            (Some(method), None) => self.notified(method, &message),
            (None, None) => self.lose("transport_failed"),
        }
    }

    fn answered(&mut self, id: &Value, message: &Value) {
        let id = id.as_str().and_then(|id| id.parse::<u64>().ok());
        let Some(pending) = id.and_then(|id| self.pending.remove(&id)) else {
            return;
        };
        let reply = match (message.get("result"), message.get("error")) {
            (Some(result), None) => Ok(result.clone()),
            (_, error) => Err(error.cloned().unwrap_or(Value::Null)),
        };
        drop(pending.reply.send(Ok(reply)));
        self.health.in_flight(self.pending.len());
    }

    fn notified(&self, method: &str, message: &Value) {
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        if method == EVENT
            && let Ok(event) = serde_json::from_value::<RemoteEvent>(params.clone())
        {
            self.health.seen(event.seq);
        }
        self.forward(Inbound::Notification {
            machine: self.machine.clone(),
            method: method.to_owned(),
            params,
        });
    }
}
