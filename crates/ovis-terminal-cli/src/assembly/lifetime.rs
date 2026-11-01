//! Stopping one server lifetime in reverse order: the serving generation's listeners and
//! dispatcher, attach deliveries, the runtime thread (lifecycle, holder, geometry log), then
//! the logs. The report adds up the listener of every boot the lifetime served.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::Arc;

use ovis_terminal_core::events::{Coordinator, EventData, SnapshotProducer};
use ovis_terminal_server::listen::ServeReport;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::AssemblyError;
use super::actor::ActorThread;
use super::generation::{Ending, Generation, Shared};

/// Everything a running server owns that must be released in order.
pub(super) struct Lifetime {
    pub(super) generation: Generation,
    pub(super) shared: Shared,
    pub(super) thread: ActorThread,
    /// Reports of the listeners earlier boots of this lifetime served.
    pub(super) reports: Vec<ServeReport>,
}

/// Closes `log`, which nothing else may hold any more.
pub(super) async fn close<D, P>(
    log: Arc<Coordinator<D, P>>,
    stage: &'static str,
) -> Result<(), AssemblyError>
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

/// The report line over the listeners of every boot.
fn summary(reports: &[ServeReport]) -> Value {
    let mut failures: BTreeMap<&'static str, u64> = BTreeMap::new();
    let (mut accepted, mut panicked) = (0_u64, 0_u64);
    for report in reports {
        accepted = accepted.saturating_add(report.accepted());
        panicked = panicked.saturating_add(report.panicked());
        for (kind, count) in report.failures() {
            let total = failures.entry(kind).or_default();
            *total = total.saturating_add(*count);
        }
    }
    let failures: serde_json::Map<String, Value> = failures
        .into_iter()
        .map(|(kind, count)| (kind.to_owned(), json!(count.to_string())))
        .collect();
    json!({"report": {
        "accepted": accepted.to_string(),
        "failures": failures,
        "panicked": panicked.to_string(),
    }})
}

impl Lifetime {
    /// Stops serving after `waited` ended, and returns the serve report line.
    pub(super) async fn stop(
        self,
        waited: Result<(), AssemblyError>,
    ) -> Result<Value, AssemblyError> {
        let Self {
            generation,
            shared,
            thread,
            mut reports,
        } = self;
        let (report, log) = generation.end(Ending::Shutdown).await?;
        reports.push(report);
        waited?;
        shared
            .lane
            .shutdown()
            .await
            .map_err(AssemblyError::stop("end attach deliveries"))?;
        let journal = Arc::clone(&shared.journal);
        drop(shared);
        tokio::task::spawn_blocking(move || thread.join())
            .await
            .map_err(AssemblyError::stop("join the runtime thread"))??;
        close(log, "close the dispatch log").await?;
        tokio::task::spawn_blocking(move || journal.close())
            .await
            .map_err(AssemblyError::stop("close the lease log"))??;
        Ok(summary(&reports))
    }
}
