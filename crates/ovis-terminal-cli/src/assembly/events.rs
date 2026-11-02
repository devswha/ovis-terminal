//! `events.subscribe`, `events.update`, `events.unsubscribe` and `events.ack` over T81's subscription
//! hub on the boot's dispatch log. Each subscription is pulled by its own blocking task
//! that writes `event`, `heartbeat` and `missed` notifications on the subscribing
//! connection; the task ends when the subscription closes, the hub stops or the
//! connection goes. Only the subscribing principal may update or close a subscription.
//! A requested snapshot is the terminal list (terminal records live in the control's
//! registry, not in this log), read before the subscription starts at the log's head.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_core::events::{Event, ResourceId};
use ovis_terminal_core::health::Health;
use ovis_terminal_core::ops::OperationProducer;
use ovis_terminal_core::registry::RuntimeIdentity;
use ovis_terminal_core::subscriptions::{
    Feed, Pulled, QueueLimits, Request, Selector, SubscriptionError, Subscriptions,
};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::events::{EventEnvelope, Heartbeat, MissedNotice};
use ovis_terminal_protocol::schema::ids::{ClientId, SubscriptionId};
use ovis_terminal_protocol::schema::methods::events::{
    EventsCursorResult, EventsSubscribeParams, EventsSubscribeResult, EventsUnsubscribeParams,
    EventsUpdateParams, EventsUpdateResult,
};
use ovis_terminal_server::conn::{Call, Notifier};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::AssemblyError;
use super::error::{internal, refusal};
use super::handler::{Served, decode};
use super::lanes::{DispatchLog, ServedEvent};

mod ack;

type Hub = Subscriptions<ServedEvent, OperationProducer>;

/// Bound on one pull; the hub answers a heartbeat sooner when nothing matched.
const PULL: Duration = Duration::from_secs(60);

fn subscription_refusal(error: &SubscriptionError) -> BoundaryError {
    refusal(error.kind(), error.hint())
}

fn checked<T: DeserializeOwned + serde::Serialize>(value: Value) -> Result<Value, BoundaryError> {
    let typed: T = serde_json::from_value(value).map_err(internal("encode an events record"))?;
    serde_json::to_value(typed).map_err(internal("encode an events record"))
}

/// The subscription hub of one boot and who owns each subscription.
#[derive(Debug)]
pub(super) struct EventLane {
    identity: RuntimeIdentity,
    hub: RwLock<Option<Hub>>,
    owners: Mutex<BTreeMap<SubscriptionId, ack::Owned>>,
}

impl EventLane {
    /// Starts the hub on `log`, reporting backlog and losses to `health`.
    pub(super) fn start(
        log: &Arc<DispatchLog>,
        identity: RuntimeIdentity,
        health: Arc<Health>,
    ) -> Result<Arc<Self>, AssemblyError> {
        let limits = QueueLimits::v1().map_err(AssemblyError::start("bound the subscriptions"))?;
        let hub = Subscriptions::start(Arc::clone(log), identity, limits)
            .map_err(AssemblyError::start("start the subscription hub"))?;
        hub.report_to(health);
        Ok(Arc::new(Self {
            identity,
            hub: RwLock::new(Some(hub)),
            owners: Mutex::new(BTreeMap::new()),
        }))
    }

    /// Stops the hub; every feed then ends `coordinator_stopped`.
    pub(super) fn shutdown(&self) -> Result<(), AssemblyError> {
        let hub = self
            .hub
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        hub.map_or(Ok(()), |hub| {
            hub.shutdown()
                .map_err(AssemblyError::stop("stop the subscription hub"))
        })
    }

    fn with_hub<R>(
        &self,
        use_it: impl FnOnce(&Hub) -> Result<R, SubscriptionError>,
    ) -> Result<R, BoundaryError> {
        let guard = self.hub.read().unwrap_or_else(PoisonError::into_inner);
        let hub = guard
            .as_ref()
            .ok_or_else(|| refusal("unavailable", "the server is restarting; subscribe again"))?;
        let used = use_it(hub).map_err(|error| subscription_refusal(&error));
        drop(guard);
        used
    }

    fn owned(&self, id: &SubscriptionId, principal: &PrincipalId) -> Result<(), BoundaryError> {
        let owners = self.owners.lock().unwrap_or_else(PoisonError::into_inner);
        match owners.get(id) {
            Some(owned) if owned.principal == *principal => Ok(()),
            _ => Err(refusal(
                "not_found",
                "name a subscription this principal opened",
            )),
        }
    }

    fn forget(&self, id: &SubscriptionId) {
        self.owners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id);
    }

    fn envelope(
        &self,
        id: &SubscriptionId,
        event: &Event<ServedEvent>,
    ) -> Result<Value, BoundaryError> {
        let terminal = event.affected.iter().find_map(|resource| match resource {
            ResourceId::Terminal(terminal) => Some(terminal.to_string()),
            _ => None,
        });
        let at = super::actor::now().map_err(internal("read the clock"))?;
        let data = event.data.payload().map_err(internal("encode an event"))?;
        checked::<EventEnvelope>(json!({
            "seq": event.cursor.seq, "lifetime": self.identity.runtime_id().to_string(),
            "boot_id": event.boot_id.to_string(),
            "machine_id": self.identity.machine_id().to_string(), "subscription_id": id,
            "ts_ms": at, "type": event.event_type,
            "target": {"terminal_id": terminal, "terminal_incarnation": null,
                "process_incarnation": null, "agent_id": null, "prompt_id": null,
                "lease_id": null, "stream_id": null, "machine_id": null},
            "data": data,
        }))
    }

    /// The notifications one pull produced, or `None` once the subscription ended.
    fn notifications(
        &self,
        id: &SubscriptionId,
        pulled: Pulled<ServedEvent, ovis_terminal_core::ops::OperationSnapshot>,
    ) -> Option<Vec<(&'static str, Value)>> {
        match pulled {
            Pulled::Events(batch) => batch
                .events
                .iter()
                .map(|event| self.envelope(id, event).ok().map(|value| ("event", value)))
                .collect(),
            Pulled::Heartbeat(beat) => checked::<Heartbeat>(json!({
                "subscription_id": beat.subscription_id, "seq": beat.seq,
                "scan_seq": beat.scan_seq, "lifetime": beat.runtime_id.to_string(),
                "boot_id": beat.boot_id.to_string()}))
            .ok()
            .map(|value| vec![("heartbeat", value)]),
            Pulled::Missed(recovery) => checked::<MissedNotice>(json!({
                "subscription_id": recovery.loss.subscription_id,
                "from_seq": recovery.loss.from_seq, "reason": recovery.loss.cause}))
            .ok()
            .map(|value| vec![("missed", value)]),
            _ => None,
        }
    }

    /// Pulls `feed` until it ends, writing each pull's notifications on `notifier`.
    fn forward(self: Arc<Self>, feed: &Feed<ServedEvent, OperationProducer>, notifier: &Notifier) {
        let id = feed.id().clone();
        let runtime = tokio::runtime::Handle::current();
        while let Some(deadline) = Instant::now().checked_add(PULL) {
            let Some(notifications) = feed.next(deadline).ok().and_then(|pulled| {
                self.advance(&id, &pulled);
                self.notifications(&id, pulled)
            }) else {
                break;
            };
            let sent = notifications
                .into_iter()
                .try_for_each(|(method, value)| runtime.block_on(notifier.notify(method, value)));
            if sent.is_err() {
                // The connection is gone; closing is best effort, the feed's drop ends it too.
                match self.with_hub(|hub| hub.unsubscribe(&id)) {
                    Ok(_) | Err(_) => {}
                }
                break;
            }
        }
        self.forget(&id);
    }
}

impl Served {
    /// `events.subscribe`, `events.update` and `events.unsubscribe`.
    pub(super) async fn events(&self, call: Call) -> Result<Value, BoundaryError> {
        let principal = call.session.principal().clone();
        match call.request.method() {
            "events.subscribe" => self.subscribe(call, principal).await,
            "events.ack" => self.ack(&call, &principal),
            "events.update" => {
                let params: EventsUpdateParams = decode(&call)?;
                self.events.owned(&params.subscription_id, &principal)?;
                let selector = Selector::decode(&params.filter)
                    .map_err(|error| subscription_refusal(&error))?;
                let seq = self
                    .events
                    .with_hub(|hub| hub.update(&params.subscription_id, selector))?;
                checked::<EventsUpdateResult>(json!({
                    "subscription_id": params.subscription_id, "effective_seq": seq}))
            }
            _ => {
                let params: EventsUnsubscribeParams = decode(&call)?;
                self.events.owned(&params.subscription_id, &principal)?;
                let closed = self
                    .events
                    .with_hub(|hub| hub.unsubscribe(&params.subscription_id))?;
                self.events.forget(&params.subscription_id);
                checked::<EventsCursorResult>(json!({
                    "seq": closed.seq, "scan_seq": closed.scan_seq}))
            }
        }
    }

    async fn subscribe(&self, call: Call, principal: PrincipalId) -> Result<Value, BoundaryError> {
        let params: EventsSubscribeParams = decode(&call)?;
        let snapshot = if params.snapshot && params.cursor.is_none() {
            let session = Arc::clone(&call.session);
            let listed = self
                .actor
                .call(move |runtime| runtime.list_terminals(&session))
                .await?;
            listed.get("terminals").cloned()
        } else {
            None
        };
        let mut request = Request::decode(&params, self.events.identity.machine_id())
            .map_err(|error| subscription_refusal(&error))?;
        if params.snapshot && params.cursor.is_none() {
            request.start = ovis_terminal_core::subscriptions::Start::Head;
        }
        let request = request.for_client(ClientId::new(principal.to_string()));
        let started = self.events.with_hub(|hub| hub.subscribe(request))?;
        let id = started.feed.id().clone();
        self.events
            .owners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                id.clone(),
                ack::Owned {
                    principal,
                    cursor: ack::Cursor::at(started.snapshot_seq.get()),
                },
            );
        let limits = started.limits;
        let result = checked::<EventsSubscribeResult>(json!({
            "subscription_id": id, "snapshot": snapshot,
            "snapshot_seq": started.snapshot_seq, "replay_floor": started.replay_floor,
            "missed": started.missed,
            "queue_events": u32::try_from(limits.queue_events).unwrap_or(u32::MAX),
            "queue_bytes": limits.queue_bytes.to_string(),
            "heartbeat_ms": limits.heartbeat.as_millis().to_string(),
        }))?;
        let (lane, feed, notifier) = (Arc::clone(&self.events), started.feed, call.notifier);
        tokio::task::spawn_blocking(move || lane.forward(&feed, &notifier));
        Ok(result)
    }
}
