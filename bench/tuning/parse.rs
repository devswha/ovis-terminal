//! Parse-budget tuning: how long one `alacritty_terminal` turn takes per budget, against
//! the unbudgeted single feed the actor would otherwise be blocked on.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ovis_terminal_vt::{Geometry, HistoryLimit, ParseBudget, PendingCap, Shard, Vt, VtConfig};
use serde::Serialize;

use super::corpus;
use super::error::TuningError;

const BUDGETS: [usize; 5] = [4 << 10, 16 << 10, 64 << 10, 256 << 10, 1 << 20];
const CORPUS_BYTES: usize = 8 << 20;

#[derive(Debug, Serialize)]
pub(super) struct TurnRow {
    corpus: &'static str,
    budget_bytes: Option<usize>,
    turns: usize,
    max_turn_ms: f64,
    mean_turn_ms: f64,
    ns_per_byte: f64,
}

fn engine(geometry: Geometry) -> Vt {
    Vt::new(VtConfig::new(geometry, HistoryLimit::new(2_000)))
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

fn float(count: usize) -> f64 {
    u32::try_from(count.max(1)).map_or_else(|_| f64::from(u32::MAX), f64::from)
}

fn per_byte(total: Duration, bytes: usize) -> f64 {
    total.as_secs_f64() * 1e9 / float(bytes)
}

fn budgeted(
    (name, bytes): (&'static str, &Arc<[u8]>),
    budget: usize,
    geometry: Geometry,
) -> TurnRow {
    let budget = ParseBudget::new(NonZeroUsize::new(budget).unwrap_or(NonZeroUsize::MIN));
    let mut shard = Shard::new(engine(geometry), budget, PendingCap::new(bytes.len()));
    let mut max = Duration::ZERO;
    let mut total = Duration::ZERO;
    let mut turns = 0_usize;
    if shard.push(Arc::clone(bytes)).is_ok() {
        while shard.pending_bytes() > 0 {
            let turn = shard.turn();
            max = max.max(turn.elapsed);
            total = total.saturating_add(turn.elapsed);
            turns = turns.saturating_add(1);
        }
    }
    TurnRow {
        corpus: name,
        budget_bytes: Some(budget.bytes()),
        turns,
        max_turn_ms: ms(max),
        mean_turn_ms: ms(total) / float(turns),
        ns_per_byte: per_byte(total, bytes.len()),
    }
}

fn unbudgeted(name: &'static str, bytes: &[u8], geometry: Geometry) -> TurnRow {
    let mut vt = engine(geometry);
    let started = Instant::now();
    vt.feed(bytes);
    let elapsed = started.elapsed();
    TurnRow {
        corpus: name,
        budget_bytes: None,
        turns: 1,
        max_turn_ms: ms(elapsed),
        mean_turn_ms: ms(elapsed),
        ns_per_byte: per_byte(elapsed, bytes.len()),
    }
}

/// Every corpus, unbudgeted first (the "before"), then each budget (the "after").
pub(super) fn run() -> Result<Vec<TurnRow>, TuningError> {
    let geometry = Geometry::new(120, 40)?;
    let corpora: [(&'static str, Vec<u8>); 3] = [
        ("build-log", corpus::build_log(CORPUS_BYTES)),
        ("sgr-every-cell", corpus::sgr_dense(CORPUS_BYTES)),
        ("wide-and-combining", corpus::wide(CORPUS_BYTES)),
    ];
    let mut rows = Vec::new();
    for (name, bytes) in corpora {
        rows.push(unbudgeted(name, &bytes, geometry));
        let shared: Arc<[u8]> = Arc::from(bytes);
        for budget in BUDGETS {
            rows.push(budgeted((name, &shared), budget, geometry));
        }
    }
    Ok(rows)
}
