//! The local latency profile: concurrent terminals under a paced input load.

use std::thread;
use std::time::{Duration, Instant};

use ovis_terminal_probes::platform::{ByteCount, Micros};
use serde::Serialize;

use super::error::HarnessError;
use super::histogram::{LatencyHistogram, LatencySummary};
use super::memory::ResidentMemory;
use super::terminal::{EchoTerminal, load_lines};

/// Load written per sample: 64 lines of 64 bytes.
const LOAD_LINES: usize = 64;
/// Bound on any single wait for a terminal.
const STEP_BOUND: Duration = Duration::from_secs(10);

/// How many terminals run, how much input they receive and the verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct LatencyProfile {
    /// Concurrent terminals, each driven by its own thread.
    pub terminals: u16,
    /// Round trips measured per terminal.
    pub samples_per_terminal: u16,
    /// Aggregate input rate across all terminals.
    pub input_bytes_per_second: u64,
    /// Every sample must stay at or below this.
    pub verdict: Duration,
}

impl LatencyProfile {
    /// The plan's local profile: 16 terminals, 1 MiB/s aggregate, 100 ms.
    pub const LOCAL: Self = Self {
        terminals: 16,
        samples_per_terminal: 20,
        input_bytes_per_second: 1024 * 1024,
        verdict: Duration::from_millis(100),
    };
}

/// Measured outcome of a latency run.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct LatencyReport {
    /// Terminals driven.
    pub terminals: u16,
    /// Requested aggregate input rate.
    pub input_bytes_per_second: ByteCount,
    /// The verdict bound.
    pub verdict: Micros,
    /// Round-trip latency over every sample.
    pub latency: LatencySummary,
    /// Whether the slowest sample stayed within the verdict.
    pub within_verdict: bool,
    /// Bytes written to all terminals.
    pub written: ByteCount,
    /// Bytes read back from all terminals.
    pub received: ByteCount,
    /// Wall time of the measured phase.
    pub elapsed: Micros,
    /// Resident memory after the measured phase.
    pub resident: ResidentMemory,
}

struct Pace {
    samples: u16,
    interval: Duration,
    load: Vec<u8>,
}

/// Runs `profile` on real terminals.
///
/// # Errors
///
/// [`HarnessError::EmptyProfile`] for a profile without terminals, samples or
/// rate; otherwise the first terminal failure.
pub fn run_latency(profile: &LatencyProfile) -> Result<LatencyReport, HarnessError> {
    let per_terminal = profile
        .input_bytes_per_second
        .checked_div(u64::from(profile.terminals))
        .filter(|rate| *rate > 0 && profile.samples_per_terminal > 0)
        .ok_or(HarnessError::EmptyProfile)?;
    let load = load_lines(LOAD_LINES);
    let load_len = u64::try_from(load.len()).unwrap_or(u64::MAX);
    let interval = load_len
        .saturating_mul(1_000_000)
        .checked_div(per_terminal)
        .map(Duration::from_micros)
        .ok_or(HarnessError::EmptyProfile)?;
    let mut terminals = (0..profile.terminals)
        .map(|index| EchoTerminal::spawn(index, STEP_BOUND))
        .collect::<Result<Vec<_>, _>>()?;
    let pace = Pace {
        samples: profile.samples_per_terminal,
        interval,
        load,
    };
    let started = Instant::now();
    let histograms = thread::scope(|scope| {
        let workers: Vec<_> = terminals
            .iter_mut()
            .map(|terminal| scope.spawn(|| drive(terminal, &pace)))
            .collect();
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect::<Result<Vec<_>, _>>()
    })?;
    let elapsed = Micros::from_duration(started.elapsed());
    let mut histogram = LatencyHistogram::default();
    for terminal in &histograms {
        histogram.merge(terminal);
    }
    let programs: Vec<_> = terminals.iter().map(EchoTerminal::program).collect();
    let resident = ResidentMemory::sample(&programs)?;
    let received = terminals
        .iter()
        .fold(ByteCount::default(), |sum, terminal| {
            sum.saturating_add(terminal.received())
        });
    let samples =
        u64::from(profile.terminals).saturating_mul(u64::from(profile.samples_per_terminal));
    for terminal in terminals {
        terminal.close(STEP_BOUND)?;
    }
    let latency = histogram.summary();
    Ok(LatencyReport {
        terminals: profile.terminals,
        input_bytes_per_second: ByteCount::new(profile.input_bytes_per_second),
        verdict: Micros::from_duration(profile.verdict),
        latency,
        within_verdict: latency.max.as_duration() <= profile.verdict,
        written: ByteCount::new(samples.saturating_mul(load_len)),
        received,
        elapsed,
        resident,
    })
}

fn drive(terminal: &mut EchoTerminal, pace: &Pace) -> Result<LatencyHistogram, HarnessError> {
    let mut histogram = LatencyHistogram::default();
    let started = Instant::now();
    for sample in 1..=u32::from(pace.samples) {
        histogram.record(terminal.round_trip(&pace.load, STEP_BOUND)?);
        // Pacing is the load profile itself: each terminal sends one load per
        // interval so the aggregate input rate matches the profile.
        let due = pace
            .interval
            .checked_mul(sample)
            .and_then(|offset| started.checked_add(offset));
        if let Some(wait) = due.map(|at| at.saturating_duration_since(Instant::now())) {
            thread::sleep(wait);
        }
    }
    Ok(histogram)
}
