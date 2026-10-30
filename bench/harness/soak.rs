//! The capacity soak: many terminals driven round-robin for a fixed duration,
//! with memory sampled once a second and process tokens compared at the end.

use std::time::{Duration, Instant};

use ovis_terminal_probes::platform::{
    ByteCount, Micros, ProcessState, SampleCount, observe_process,
};
use serde::Serialize;

use super::error::HarnessError;
use super::histogram::{LatencyHistogram, LatencySummary};
use super::memory::ResidentMemory;
use super::terminal::{EchoTerminal, load_lines};

/// Load written per round trip: 4 lines of 64 bytes.
const LOAD_LINES: usize = 4;
const STEP_BOUND: Duration = Duration::from_secs(10);
const MEMORY_EVERY: Duration = Duration::from_secs(1);

/// How many terminals run and for how long.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SoakProfile {
    /// Concurrent terminals.
    pub terminals: u16,
    /// Length of the measured phase.
    pub duration: Duration,
}

impl SoakProfile {
    /// The plan's capacity target.
    pub const CAPACITY_TERMINALS: u16 = 50;

    /// A soak of `terminals` terminals for `duration`.
    #[must_use]
    pub const fn new(terminals: u16, duration: Duration) -> Self {
        Self {
            terminals,
            duration,
        }
    }
}

/// Measured outcome of a soak.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct SoakReport {
    /// Terminals driven.
    pub terminals: u16,
    /// Completed round-robin rounds over every terminal.
    pub rounds: SampleCount,
    /// Round-trip latency over every sample.
    pub latency: LatencySummary,
    /// Memory after the terminals started.
    pub resident_first: ResidentMemory,
    /// Sample with the highest total.
    pub resident_peak: ResidentMemory,
    /// Memory at the end of the measured phase.
    pub resident_last: ResidentMemory,
    /// Programs still live with the start token they had at the beginning.
    pub programs_with_unchanged_tokens: u16,
    /// Bytes read back from all terminals.
    pub received: ByteCount,
    /// Wall time of the measured phase.
    pub elapsed: Micros,
}

/// Runs `profile` on real terminals.
///
/// # Errors
///
/// [`HarnessError::EmptyProfile`] for zero terminals or duration; otherwise
/// the first terminal or process-probe failure.
pub fn run_soak(profile: &SoakProfile) -> Result<SoakReport, HarnessError> {
    if profile.terminals == 0 || profile.duration.is_zero() {
        return Err(HarnessError::EmptyProfile);
    }
    let mut terminals = (0..profile.terminals)
        .map(|index| EchoTerminal::spawn(index, STEP_BOUND))
        .collect::<Result<Vec<_>, _>>()?;
    let programs: Vec<_> = terminals.iter().map(EchoTerminal::program).collect();
    let before = programs
        .iter()
        .map(|pid| observe_process(*pid))
        .collect::<Result<Vec<_>, _>>()?;
    let load = load_lines(LOAD_LINES);
    let resident_first = ResidentMemory::sample(&programs)?;
    let mut resident_peak = resident_first;
    let mut histogram = LatencyHistogram::default();
    let mut rounds = 0_u64;
    let started = Instant::now();
    let mut sampled = started;
    while started.elapsed() < profile.duration {
        for terminal in &mut terminals {
            histogram.record(terminal.round_trip(&load, STEP_BOUND)?);
        }
        rounds = rounds.saturating_add(1);
        if sampled.elapsed() >= MEMORY_EVERY {
            sampled = Instant::now();
            let sample = ResidentMemory::sample(&programs)?;
            if sample.total() > resident_peak.total() {
                resident_peak = sample;
            }
        }
    }
    let elapsed = Micros::from_duration(started.elapsed());
    let resident_last = ResidentMemory::sample(&programs)?;
    let mut unchanged = 0_u16;
    for earlier in &before {
        let now = observe_process(earlier.pid)?;
        if now.state == ProcessState::Live && now.start_token == earlier.start_token {
            unchanged = unchanged.saturating_add(1);
        }
    }
    let received = terminals
        .iter()
        .fold(ByteCount::default(), |sum, terminal| {
            sum.saturating_add(terminal.received())
        });
    for terminal in terminals {
        terminal.close(STEP_BOUND)?;
    }
    Ok(SoakReport {
        terminals: profile.terminals,
        rounds: SampleCount::new(rounds),
        latency: histogram.summary(),
        resident_first,
        resident_peak: if resident_last.total() > resident_peak.total() {
            resident_last
        } else {
            resident_peak
        },
        resident_last,
        programs_with_unchanged_tokens: unchanged,
        received,
        elapsed,
    })
}
