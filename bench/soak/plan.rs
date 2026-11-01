//! What one run does and when: the disruption schedule and the limits it is judged by.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A deliberate disruption of the running runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind {
    /// SIGKILL of the control process, then a crash-recovery boot.
    CoreCrash,
    /// A planned handover to a new control boot.
    PlannedRestart,
    /// The observing client's connection is cut and resumed from its last sequence.
    RemoteInterruption,
    /// A viewer that connects and never reads.
    StalledViewer,
}

impl Kind {
    pub(super) const ALL: [Self; 4] = [
        Self::CoreCrash,
        Self::PlannedRestart,
        Self::RemoteInterruption,
        Self::StalledViewer,
    ];

    /// Shift of this kind's slots, so kinds with equal counts never coincide.
    const fn shift(self) -> Duration {
        Duration::from_secs(match self {
            Self::CoreCrash => 0,
            Self::PlannedRestart => 3,
            Self::RemoteInterruption => 6,
            Self::StalledViewer => 9,
        })
    }
}

/// One disruption at `at` after the start of the measured run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct Scheduled {
    #[serde(serialize_with = "millis")]
    pub(super) at: Duration,
    pub(super) kind: Kind,
}

fn millis<S: serde::Serializer>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.as_millis().to_string())
}

/// Shortest spacing between two disruptions.
const SPACING: Duration = Duration::from_secs(2);

/// Spreads `count` disruptions of each kind over `[start, end)`: each kind takes the
/// midpoints of `count` equal slices, shifted by kind, then the list is spaced by at
/// least [`SPACING`]. `None` when they cannot all fit before `end`.
pub(super) fn schedule(
    counts: &[(Kind, usize)],
    (start, end): (Duration, Duration),
) -> Option<Vec<Scheduled>> {
    let span = end.checked_sub(start)?;
    let mut list = Vec::new();
    for &(kind, count) in counts {
        let slices = u32::try_from(count).ok()?.checked_mul(2)?;
        for slot in 0..count {
            let odd = u32::try_from(slot).ok()?.checked_mul(2)?.checked_add(1)?;
            let at = start
                .checked_add(span.checked_mul(odd)?.checked_div(slices)?)?
                .checked_add(kind.shift())?;
            list.push(Scheduled { at, kind });
        }
    }
    list.sort_by_key(|item| (item.at, item.kind));
    let mut earliest = start;
    for item in &mut list {
        item.at = item.at.max(earliest);
        earliest = item.at.checked_add(SPACING)?;
    }
    list.iter().all(|item| item.at < end).then_some(list)
}

/// The fault fixture of the failure case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "fixture", rename_all = "kebab-case", deny_unknown_fields)]
pub(super) enum Fixture {
    /// Terminal 0 keeps one stale checkpoint while its holder tail rolls past it, and an
    /// input's receipt is lost in a crash: recovery must report a gap, an inexact screen
    /// and `outcome_unknown`, and must not send the input again.
    HolderTailGapAndLostReceipt,
}

/// The bounds a run is judged against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct Limits {
    pub(super) rss_bytes: u64,
    pub(super) disk_bytes: u64,
    /// Every control reply (ping) while a boot serves.
    pub(super) control_ms: u64,
    /// Bytes one viewer's queue may hold before it is cut.
    pub(super) queue_bytes: u64,
    /// Growth allowed after warmup: this many bytes plus `growth_percent` of the baseline.
    pub(super) growth_bytes: u64,
    pub(super) growth_percent: u64,
}

/// One validated run.
#[derive(Debug, Clone)]
pub(super) struct Plan {
    pub(super) label: &'static str,
    pub(super) terminals: usize,
    pub(super) duration: Duration,
    pub(super) warmup: Duration,
    /// Length of the baseline window after warmup and of the final window.
    pub(super) window: Duration,
    pub(super) disruptions: Vec<Scheduled>,
    pub(super) counts: Vec<(Kind, usize)>,
    pub(super) holder_loss_probe: bool,
    pub(super) bytes_per_terminal: usize,
    pub(super) checkpoint_every: Duration,
    pub(super) history_lines: u64,
    pub(super) limits: Limits,
    pub(super) fixture: Option<Fixture>,
    /// The private directory every runtime file of the run lives under.
    pub(super) root: PathBuf,
    pub(super) evidence: Option<PathBuf>,
}
