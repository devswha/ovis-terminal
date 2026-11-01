//! The status feed: every committed `status.changed` of the current boot's dispatch log
//! reaches the reporting terminal's wait hub, so turn and state waits see declared
//! status. The runtime owns the subscription and drains it on its own thread between
//! jobs, as it does holder messages; each generation installs its own.

use std::io::Write as _;
use std::sync::mpsc::TryRecvError;

use ovis_terminal_agent::status::AgentStatusEvent;
use ovis_terminal_core::events::{Delivery, EventError, ResourceId};
use ovis_terminal_protocol::schema::scalar::EventSeq;

use super::AssemblyError;
use super::actor::Runtime;
use super::lanes::{DispatchLog, ServedEvent};

/// Subscribes to `log` from its current position.
pub(super) fn subscribe(log: &DispatchLog) -> Result<Delivery<ServedEvent>, AssemblyError> {
    Ok(log
        .subscribe(None)
        .map_err(AssemblyError::start("subscribe the status feed"))?
        .delivery)
}

fn warn(detail: &EventError) {
    let line = serde_json::json!({"warning": {"stage": "feed declared status to waits",
        "detail": detail.to_string()}});
    match writeln!(std::io::stderr().lock(), "{line}") {
        Ok(()) | Err(_) => {}
    }
}

impl Runtime {
    /// Applies every status event delivered so far; a closed or lost feed is dropped.
    pub(super) fn drain_status(&mut self) {
        loop {
            let Some(feed) = &self.status_feed else {
                return;
            };
            let event = match feed.try_recv() {
                Ok(event) => event,
                Err(EventError::TryReceive(TryRecvError::Empty)) => return,
                Err(EventError::TryReceive(TryRecvError::Disconnected)) => {
                    self.status_feed = None;
                    return;
                }
                Err(error) => {
                    warn(&error);
                    self.status_feed = None;
                    return;
                }
            };
            let ServedEvent::Agent(AgentStatusEvent::StatusChanged(snapshot)) = &event.data else {
                continue;
            };
            let seq = EventSeq::new(event.cursor.seq.get());
            for resource in &event.affected {
                if let ResourceId::Terminal(terminal) = *resource {
                    self.feed_status(terminal, (snapshot, seq));
                }
            }
        }
    }
}
