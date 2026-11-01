//! Request lanes: each keeps one request in flight, so `inflight` lanes put that many
//! requests on the actors at once. Admission samples time the receipt only (EXCL-024);
//! a rejection is an answered request and is sampled too, never dropped.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use ovis_terminal_server::streams::scheduler::TerminalHandle;
use ovis_terminal_vt::Geometry;

/// Every lane issues one request per tick.
const REQUEST_TICK: Duration = Duration::from_millis(10);

/// Shared sample accounting of one run.
#[derive(Debug)]
pub(super) struct Progress {
    taken: AtomicUsize,
    warmup: usize,
    wanted: usize,
    until: Instant,
    measuring_from: OnceLock<Instant>,
}

impl Progress {
    pub(super) fn new(warmup: usize, samples: usize, max_run: Duration) -> Self {
        let now = Instant::now();
        let measuring_from = OnceLock::new();
        if warmup == 0 {
            measuring_from.get_or_init(|| now);
        }
        Self {
            taken: AtomicUsize::new(0),
            warmup,
            wanted: warmup.saturating_add(samples),
            until: now.checked_add(max_run).unwrap_or(now),
            measuring_from,
        }
    }

    /// Instant the first measured sample started; earlier commits are warmup.
    pub(super) fn measuring_from(&self) -> Option<Instant> {
        self.measuring_from.get().copied()
    }

    pub(super) fn taken(&self) -> usize {
        self.taken.load(Ordering::Acquire)
    }

    pub(super) const fn wanted(&self) -> usize {
        self.wanted
    }

    fn running(&self) -> bool {
        self.taken() < self.wanted && Instant::now() < self.until
    }

    /// Claims the next sample slot; true when it is a warmup sample.
    fn claim(&self, started: Instant) -> bool {
        let slot = self.taken.fetch_add(1, Ordering::AcqRel);
        if slot == self.warmup {
            self.measuring_from.get_or_init(|| started);
        }
        slot < self.warmup
    }
}

/// What one lane recorded.
#[derive(Debug, Default)]
pub(super) struct LaneSamples {
    pub(super) warmup: Vec<Duration>,
    pub(super) admission: Vec<Duration>,
    pub(super) control: Vec<Duration>,
    pub(super) admitted: u64,
    pub(super) rejected: BTreeMap<&'static str, u64>,
}

impl LaneSamples {
    pub(super) fn absorb(&mut self, other: Self) {
        self.warmup.extend(other.warmup);
        self.admission.extend(other.admission);
        self.control.extend(other.control);
        self.admitted = self.admitted.saturating_add(other.admitted);
        for (kind, count) in other.rejected {
            let entry = self.rejected.entry(kind).or_default();
            *entry = entry.saturating_add(count);
        }
    }

    fn admission(&mut self, (warmup, sample): (bool, Duration)) {
        if warmup {
            self.warmup.push(sample);
        } else {
            self.admission.push(sample);
        }
    }
}

/// Admissions, resizes and cancels across `handles` until every sample is taken.
pub(super) async fn lane(
    index: usize,
    handles: Arc<[TerminalHandle]>,
    progress: Arc<Progress>,
) -> LaneSamples {
    let mut samples = LaneSamples::default();
    let mut pace = tokio::time::interval(REQUEST_TICK);
    let mut last = None;
    let mut round = 0_usize;
    let geometries = [Geometry::new(100, 40).ok(), Geometry::new(120, 40).ok()];
    while progress.running() {
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
                let answer = match handle.reserve(8) {
                    Ok(held) => handle.admit(held, b"echo ok\r".to_vec()).await,
                    Err(refused) => Err(refused),
                };
                let warmup = progress.claim(started);
                samples.admission((warmup, started.elapsed()));
                match answer {
                    Ok(receipt) => {
                        last = Some(receipt.ticket);
                        samples.admitted = samples.admitted.saturating_add(1);
                    }
                    Err(refused) => {
                        let entry = samples.rejected.entry(refused.kind()).or_default();
                        *entry = entry.saturating_add(1);
                    }
                }
            }
            2 => {
                let geometry = geometries.get(round % 8 / 4).copied().flatten();
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
