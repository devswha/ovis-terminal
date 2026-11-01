//! Stopping one server lifetime in reverse order: listeners, attach deliveries, dispatched
//! operations, the runtime thread (lifecycle, holder, geometry log), then the logs.

use std::fmt::Debug;
use std::sync::Arc;

use ovis_terminal_core::events::{Coordinator, EventData, SnapshotProducer};
use ovis_terminal_server::authn::Revalidated;
use ovis_terminal_server::listen::ServerHandle;
use ovis_terminal_server::streams::AttachLane;
use ovis_terminal_server::ws::WsServerHandle;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::AssemblyError;
use super::actor::ActorThread;
use super::boot::LeaseLog;
use super::directory::Directory;
use super::handler::Assembled;
use super::lanes::{Dispatch, DispatchLog};

/// Everything a running server owns that must be released in order.
pub(super) struct Lifetime {
    pub(super) websocket: Option<WsServerHandle>,
    pub(super) listener: ServerHandle,
    pub(super) lane: Arc<AttachLane<Arc<Directory>>>,
    pub(super) dispatch: Arc<Dispatch>,
    pub(super) handler: Arc<Revalidated<Assembled>>,
    pub(super) thread: ActorThread,
    pub(super) logs: (Arc<DispatchLog>, Arc<LeaseLog>),
}

async fn close<D, P>(log: Arc<Coordinator<D, P>>, stage: &'static str) -> Result<(), AssemblyError>
where
    D: EventData + Clone + Serialize + DeserializeOwned + Send + Sync + Debug + 'static,
    P: SnapshotProducer + Send + Sync + Debug + 'static,
{
    let log = Arc::try_unwrap(log).map_err(|_| AssemblyError::stop(stage)("still shared"))?;
    tokio::task::spawn_blocking(move || log.close())
        .await
        .map_err(AssemblyError::stop(stage))?
        .map_err(AssemblyError::stop(stage))
}

impl Lifetime {
    /// Stops serving after `waited` ended, and returns the serve report line.
    pub(super) async fn stop(
        self,
        waited: Result<(), AssemblyError>,
    ) -> Result<Value, AssemblyError> {
        if let Some(websocket) = self.websocket {
            websocket
                .shutdown()
                .await
                .map_err(AssemblyError::stop("close the WebSocket listener"))?;
        }
        let report = self
            .listener
            .shutdown()
            .await
            .map_err(AssemblyError::stop("close the endpoint"))?;
        waited?;
        self.lane
            .shutdown()
            .await
            .map_err(AssemblyError::stop("end attach deliveries"))?;
        self.dispatch
            .shutdown()
            .await
            .map_err(AssemblyError::stop("join dispatched operations"))?;
        drop((self.dispatch, self.handler, self.lane));
        let thread = self.thread;
        tokio::task::spawn_blocking(move || thread.join())
            .await
            .map_err(AssemblyError::stop("join the runtime thread"))??;
        let (dispatch_log, leases) = self.logs;
        close(dispatch_log, "close the dispatch log").await?;
        close(leases, "close the lease log").await?;
        let failures: serde_json::Map<String, Value> = report
            .failures()
            .iter()
            .map(|(kind, count)| ((*kind).to_owned(), json!(count.to_string())))
            .collect();
        Ok(json!({"report": {
            "accepted": report.accepted().to_string(),
            "failures": failures,
            "panicked": report.panicked().to_string(),
        }}))
    }
}
