//! A dispatch collaborator for connection tests: ordered queries, a mutation that
//! publishes to subscribers, and a request that holds until the connection cancels it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_server::conn::{Call, Notifier, RequestHandler};
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};

use super::support::TestResult;

/// Records what the connection dispatched to it.
#[derive(Debug, Default)]
pub struct Recorder {
    calls: AtomicU64,
    mutations: AtomicU64,
    cancelled: AtomicU64,
    released: Notify,
    held: watch::Sender<u64>,
    subscribers: Mutex<Vec<(String, Notifier)>>,
}

fn internal<E>(_: E) -> BoundaryError {
    BoundaryError::domain(ErrorKind::Internal, false)
}

impl Recorder {
    /// Requests dispatched.
    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }

    /// Mutations committed.
    pub fn mutations(&self) -> u64 {
        self.mutations.load(Ordering::SeqCst)
    }

    /// Held requests whose futures were dropped.
    pub fn cancelled(&self) -> u64 {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Resolves once `count` held requests are running.
    ///
    /// # Errors
    ///
    /// Fails when the recorder is dropped first.
    pub async fn wait_held(&self, count: u64) -> TestResult {
        let mut held = self.held.subscribe();
        held.wait_for(|held| *held >= count).await?;
        Ok(())
    }

    async fn get(&self, name: &str) -> Value {
        match name {
            // `a` cannot finish before `b` ran: only concurrent dispatch settles both.
            "a" => self.released.notified().await,
            "b" => self.released.notify_one(),
            "hold" => self.hold().await,
            _ => {}
        }
        json!({ "name": name })
    }

    async fn hold(&self) {
        let _cancelled = CountOnDrop(&self.cancelled);
        self.held.send_modify(|held| *held = held.saturating_add(1));
        std::future::pending::<()>().await;
    }

    async fn set(&self) -> Result<Value, BoundaryError> {
        let version = self
            .mutations
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        let subscribers = self
            .subscribers
            .lock()
            .map_err(internal::<PoisonError<_>>)?
            .clone();
        for (subscription_id, notifier) in subscribers {
            let event = json!({
                "subscription_id": subscription_id,
                "seq": version.to_string(),
                "type": "doc.changed",
            });
            notifier.notify("event", event).await.map_err(internal)?;
        }
        Ok(json!({ "version": version.to_string() }))
    }

    fn subscribe(&self, notifier: Notifier) -> Result<Value, BoundaryError> {
        let mut subscribers = self
            .subscribers
            .lock()
            .map_err(internal::<PoisonError<_>>)?;
        let subscription_id = format!("sub-{}", subscribers.len().saturating_add(1));
        subscribers.push((subscription_id.clone(), notifier));
        drop(subscribers);
        Ok(json!({ "subscription_id": subscription_id }))
    }
}

struct CountOnDrop<'a>(&'a AtomicU64);

impl Drop for CountOnDrop<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl RequestHandler for Recorder {
    fn methods(&self) -> Vec<&'static str> {
        vec!["doc.get", "doc.set", "events.subscribe"]
    }

    async fn call(&self, call: Call) -> Result<Value, BoundaryError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let name = call.request.params().get("name").and_then(Value::as_str);
        match (call.request.method(), name) {
            ("doc.get", Some(name)) => Ok(self.get(name).await),
            ("doc.set", Some(_)) => self.set().await,
            ("events.subscribe", _) => self.subscribe(call.notifier),
            _ => Err(BoundaryError::invalid_params(Some("params.name"))),
        }
    }
}
