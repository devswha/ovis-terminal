//! The fixture extension: an in-process consumer of the prompt bridge, driven by the
//! fixture-only `fixture.bridge` method.
//!
//! It stands in for an agent's widget extension such as pi-ask: it attaches with its
//! program's process token, takes typed extension events and acknowledges each by the
//! version it carries, or ends its session before acknowledging. It counts every event it
//! receives per prompt, so a second delivery of one answer would be visible.

use std::collections::BTreeMap;
use std::num::NonZeroU8;
use std::sync::{Arc, Mutex};

use ovis_terminal_agent::bridge::{BridgeError, BridgeToken, PromptBridge};
use ovis_terminal_core::events::{Coordinator, EventError};
use ovis_terminal_core::ops::OperationProducer;
use ovis_terminal_protocol::domain::prompt_bridge::ExtensionEvent;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_protocol::schema::ids::{ProcessToken, PromptId};
use ovis_terminal_protocol::schema::scalar::RecordVersion;
use ovis_terminal_store::Snapshot;
use serde::Deserialize;
use serde_json::{Value, json};

use super::handler::Event;

/// Most delivered, unacknowledged events the fixture extension's session holds.
pub(crate) const QUEUE_LIMIT: NonZeroU8 = NonZeroU8::MIN.saturating_add(1);

type Core = Coordinator<Event, OperationProducer>;

/// What the scenario asks the extension to do.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    /// Open a session with a program's process token.
    Attach { process_token: ProcessToken },
    /// Take the next events, acknowledging each at its version when `ack` is set.
    Take { ack: bool },
    /// Acknowledge one held event at `version`.
    Ack {
        prompt_id: PromptId,
        version: RecordVersion,
    },
    /// Take the next events, then end the session before acknowledging any.
    Drop,
    /// Where the delivery of `prompt_id` stands, and how often the extension received it.
    State { prompt_id: PromptId },
}

#[derive(Debug)]
pub(crate) struct Extension {
    bridge: PromptBridge,
    core: Arc<Core>,
    session: Mutex<Option<BridgeToken>>,
    received: Mutex<BTreeMap<PromptId, u64>>,
}

fn refused(error: &BridgeError) -> BoundaryError {
    BoundaryError::Domain {
        data: Box::new(error.error_data()),
    }
}

fn internal() -> BoundaryError {
    BoundaryError::domain(ErrorKind::from("internal".to_owned()), false)
        .with_hint("restart the fixture")
}

impl Extension {
    pub(crate) fn new(bridge: PromptBridge, core: Arc<Core>) -> Arc<Self> {
        Arc::new(Self {
            bridge,
            core,
            session: Mutex::new(None),
            received: Mutex::new(BTreeMap::new()),
        })
    }

    /// Runs one `fixture.bridge` request off the executor.
    pub(crate) async fn call(
        self: &Arc<Self>,
        request: &RpcRequest,
    ) -> Result<Value, BoundaryError> {
        let action: Action = decode_params(request).map_err(BoundaryError::from)?;
        let extension = Arc::clone(self);
        tokio::task::spawn_blocking(move || extension.act(action))
            .await
            .map_err(|_| internal())?
    }

    fn act(&self, action: Action) -> Result<Value, BoundaryError> {
        match action {
            Action::Attach { process_token } => {
                let attached = self
                    .bridge
                    .attach(&process_token)
                    .map_err(|e| refused(&e))?;
                *self.session.lock().map_err(|_| internal())? = Some(attached.token);
                serde_json::to_value(attached.attachment).map_err(|_| internal())
            }
            Action::Take { ack } => {
                let session = self.current()?;
                let events = self.take(&session)?;
                let mut acked = Vec::new();
                for event in events.iter().filter(|_| ack) {
                    let place = (&session, &event.prompt_id, event.version);
                    acked.push(self.read(|snapshot| self.bridge.ack(snapshot, place))?);
                }
                Ok(json!({"received": events, "acked": acked}))
            }
            Action::Ack { prompt_id, version } => {
                let session = self.current()?;
                let place = (&session, &prompt_id, version);
                let acked = self.read(|snapshot| self.bridge.ack(snapshot, place))?;
                serde_json::to_value(acked).map_err(|_| internal())
            }
            Action::Drop => {
                let session = self.current()?;
                let events = self.take(&session)?;
                self.bridge.detach(&session).map_err(|e| refused(&e))?;
                *self.session.lock().map_err(|_| internal())? = None;
                Ok(json!({"received": events}))
            }
            Action::State { prompt_id } => {
                let delivery = self.read(|snapshot| self.bridge.delivery(snapshot, &prompt_id))?;
                let received = self.received.lock().map_err(|_| internal())?;
                let count = received.get(&prompt_id).copied().unwrap_or(0);
                drop(received);
                Ok(json!({"delivery": delivery, "received": count.to_string()}))
            }
        }
    }

    fn current(&self) -> Result<BridgeToken, BoundaryError> {
        let session = self.session.lock().map_err(|_| internal())?.clone();
        session.ok_or_else(|| refused(&BridgeError::Forbidden))
    }

    /// The session's next events, each counted as received.
    fn take(&self, session: &BridgeToken) -> Result<Vec<ExtensionEvent>, BoundaryError> {
        let events = self.read(|snapshot| self.bridge.next(snapshot, session))?;
        let mut received = self.received.lock().map_err(|_| internal())?;
        for event in &events {
            let count = received.entry(event.prompt_id.clone()).or_insert(0);
            *count = count.saturating_add(1);
        }
        drop(received);
        Ok(events)
    }

    fn read<T>(
        &self,
        read: impl FnOnce(&Snapshot) -> Result<T, BridgeError>,
    ) -> Result<T, BoundaryError> {
        self.core
            .read_view(|snapshot| Ok::<_, EventError>(read(snapshot)))
            .map_err(|_| internal())?
            .map_err(|error| refused(&error))
    }
}
