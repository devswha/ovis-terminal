//! Remote notifications mirrored into a coordinator's sequence domain, one commit each.

use std::sync::Arc;

use ovis_terminal_core::events::{Coordinator, EventData, SnapshotProducer};
use ovis_terminal_protocol::domain::machine_actor::{EVENT, HEARTBEAT, RemoteEvent, RemoteScan};
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_remote::actors::Inbound;
use ovis_terminal_remote::mirror::{Mirror, MirrorError, MirrorEvent, Mirrored};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::mpsc;

/// What a pump did before its machines' notifications ended.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PumpReport {
    /// Records committed locally.
    pub committed: u64,
    /// Remote records refused as undecodable or of another protocol; the next accepted
    /// event after one reports the skipped sequences as a gap.
    pub refused: u64,
    /// Records whose local commit failed.
    pub failed: u64,
}

fn now_ms() -> TimestampMs {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    TimestampMs::new(since.map_or(0, |since| {
        u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
    }))
}

fn records(mirror: &mut Mirror, inbound: Inbound) -> Result<Vec<Mirrored>, MirrorError> {
    match inbound {
        Inbound::Attached(machine) => Ok(vec![mirror.resumed(&machine)]),
        Inbound::Lost(machine) => Ok(vec![mirror.lost(&machine)]),
        Inbound::Notification {
            machine,
            method,
            params,
        } => match method.as_str() {
            EVENT => {
                let event: RemoteEvent =
                    serde_json::from_value(params).map_err(MirrorError::Decode)?;
                mirror.accept((&machine, now_ms()), event)
            }
            HEARTBEAT => {
                let scan: RemoteScan =
                    serde_json::from_value(params).map_err(MirrorError::Decode)?;
                mirror.scan(&machine, &scan)
            }
            _ => Ok(Vec::new()),
        },
        _ => Ok(Vec::new()),
    }
}

/// Mirrors every notification from `inbound` into `core` until the actors are gone; local
/// sequences follow arrival order and each remote event keeps its origin.
pub async fn pump<D, P>(
    mut inbound: mpsc::Receiver<Inbound>,
    core: Arc<Coordinator<D, P>>,
) -> PumpReport
where
    D: EventData + From<MirrorEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
{
    let mut mirror = Mirror::default();
    let mut report = PumpReport::default();
    while let Some(next) = inbound.recv().await {
        let Ok(batch) = records(&mut mirror, next) else {
            report.refused = report.refused.saturating_add(1);
            continue;
        };
        for record in batch {
            let core = Arc::clone(&core);
            let committed = tokio::task::spawn_blocking(move || {
                record
                    .into_commit::<D>()
                    .ok()
                    .map(|commit| core.commit(commit).is_ok())
            })
            .await;
            match committed {
                Ok(Some(true)) => report.committed = report.committed.saturating_add(1),
                Ok(None) => report.refused = report.refused.saturating_add(1),
                Ok(Some(false)) | Err(_) => report.failed = report.failed.saturating_add(1),
            }
        }
    }
    report
}
