//! The measured side of a load run: request lanes, control-event watchers and viewers.

use std::time::{Duration, Instant};

use ovis_terminal_server::streams::scheduler::{OutputEvent, TerminalEvent, TerminalHandle};
use ovis_terminal_vt::Geometry;
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

/// Every lane issues one request per tick, so all lanes are in flight together.
const REQUEST_TICK: Duration = Duration::from_millis(10);

#[derive(Debug, Default)]
pub(super) struct LaneSamples {
    pub(super) admission: Vec<Duration>,
    pub(super) control: Vec<Duration>,
    pub(super) rejected: u64,
}

/// Admissions, resizes and cancels across `handles` until `until`.
pub(super) async fn lane(
    index: usize,
    handles: Vec<TerminalHandle>,
    until: Instant,
) -> LaneSamples {
    let mut samples = LaneSamples::default();
    let mut pace = tokio::time::interval(REQUEST_TICK);
    let mut last = None;
    let mut round = 0_usize;
    let wide = Geometry::new(100, 40).ok();
    let narrow = Geometry::new(120, 40).ok();
    while Instant::now() < until {
        pace.tick().await;
        round = round.wrapping_add(1);
        let slot = index
            .wrapping_mul(7)
            .wrapping_add(round)
            .checked_rem(handles.len());
        let Some(handle) = slot.and_then(|slot| handles.get(slot)) else {
            break;
        };
        let started = Instant::now();
        match round % 4 {
            0 | 1 => {
                let admitted = match handle.reserve(8) {
                    Ok(held) => handle.admit(held, b"echo ok\r".to_vec()).await.ok(),
                    Err(_) => None,
                };
                match admitted {
                    Some(receipt) => {
                        last = Some(receipt.ticket);
                        samples.admission.push(started.elapsed());
                    }
                    None => samples.rejected = samples.rejected.saturating_add(1),
                }
            }
            2 => {
                let geometry = if round % 8 == 2 { wide } else { narrow };
                if let Some(geometry) = geometry
                    && handle.resize(geometry).await.is_ok()
                {
                    samples.control.push(started.elapsed());
                }
            }
            _ => {
                if let Some(ticket) = last
                    && handle.cancel(ticket).await.is_ok()
                {
                    samples.control.push(started.elapsed());
                }
            }
        }
    }
    samples
}

/// Commit-to-delivery latency of every committed control event; events lost to lag.
pub(super) async fn watch(mut events: Receiver<TerminalEvent>) -> (Vec<Duration>, u64) {
    let mut latency = Vec::new();
    let mut lost = 0_u64;
    loop {
        match events.recv().await {
            Ok(
                TerminalEvent::Admitted { at, .. }
                | TerminalEvent::Withdrawn { at, .. }
                | TerminalEvent::Resized { at, .. },
            ) => latency.push(at.elapsed()),
            Ok(_) => {}
            Err(RecvError::Lagged(missed)) => lost = lost.saturating_add(missed),
            Err(RecvError::Closed) => return (latency, lost),
        }
    }
}

/// A reading viewer: bytes received and events lost to lag.
pub(super) async fn view(mut output: Receiver<OutputEvent>) -> (u64, u64) {
    let mut bytes = 0_u64;
    let mut lost = 0_u64;
    loop {
        match output.recv().await {
            Ok(OutputEvent::Chunk { bytes: chunk, .. }) => {
                bytes = bytes.saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
            }
            Ok(_) => {}
            Err(RecvError::Lagged(missed)) => lost = lost.saturating_add(missed),
            Err(RecvError::Closed) => return (bytes, lost),
        }
    }
}

/// Drains a viewer that never read during the run: what it still holds and what it lost.
pub(super) fn drain_stalled(mut output: Receiver<OutputEvent>) -> (u64, u64) {
    let mut held = 0_u64;
    let mut lost = 0_u64;
    loop {
        match output.try_recv() {
            Ok(_) => held = held.saturating_add(1),
            Err(TryRecvError::Lagged(missed)) => lost = lost.saturating_add(missed),
            Err(TryRecvError::Empty | TryRecvError::Closed) => return (held, lost),
        }
    }
}

/// The largest sample in milliseconds.
pub(super) fn max_ms(samples: &[Duration]) -> f64 {
    samples
        .iter()
        .copied()
        .max()
        .unwrap_or_default()
        .as_secs_f64()
        * 1e3
}

/// The sample at `per_mille` of the sorted samples, in milliseconds.
pub(super) fn quantile_ms(samples: &mut [Duration], per_mille: usize) -> f64 {
    samples.sort_unstable();
    let index = samples.len().saturating_mul(per_mille) / 1000;
    samples
        .get(index.min(samples.len().saturating_sub(1)))
        .copied()
        .unwrap_or_default()
        .as_secs_f64()
        * 1e3
}
