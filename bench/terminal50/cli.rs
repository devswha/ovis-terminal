//! The command line and its validation into one [`Plan`].

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};

use super::error::Terminal50Error;
use super::plan::{Fixture, IDEAL_SHAPE, Plan, Profile, Shape, Source};

#[derive(Debug, Parser)]
#[command(
    name = "terminal50",
    version,
    about = "Benchmark terminal actors on real PTYs against the published latency budgets.",
    args_conflicts_with_subcommands = true,
    disable_help_subcommand = true
)]
pub(super) struct Cli {
    #[command(subcommand)]
    pub(super) mode: Option<Mode>,
    /// The ovis-terminal binary under test; its hash and version enter the evidence.
    #[arg(long)]
    binary: Option<PathBuf>,
    /// Run in a fresh private root; required.
    #[arg(long)]
    isolated: bool,
    #[arg(long)]
    terminals: Option<usize>,
    #[arg(long)]
    clients: Option<usize>,
    #[arg(long)]
    inflight: Option<usize>,
    #[arg(long)]
    output_bytes_per_second: Option<usize>,
    /// Measured admission samples, after the warmup.
    #[arg(long)]
    samples: Option<usize>,
    #[arg(long, default_value_t = 1000)]
    warmup_samples: usize,
    /// Output viewers per terminal (the first never reads); defaults to --clients.
    #[arg(long)]
    viewers_per_terminal: Option<usize>,
    /// IDEAL profile: every admission sample within this bound.
    #[arg(long)]
    admission_max_ms: Option<u64>,
    /// IDEAL profile: every event sample within this bound.
    #[arg(long)]
    event_max_ms: Option<u64>,
    /// Capacity profile: admission p99 within this bound.
    #[arg(long)]
    admission_p99_ms: Option<u64>,
    /// Capacity profile: event p99 within this bound.
    #[arg(long)]
    event_p99_ms: Option<u64>,
    #[arg(long, default_value_t = 1024)]
    rss_limit_mib: u64,
    #[arg(long, default_value_t = 4096)]
    disk_limit_mib: u64,
    /// Runtime worker threads; defaults to the host parallelism clamped to 2..=8.
    #[arg(long)]
    workers: Option<usize>,
    /// Hang guard on the whole measurement.
    #[arg(long, default_value_t = 900)]
    max_seconds: u64,
    /// A failure fixture, `{"fixture":"control-writer-delay","delay_ms":150}`.
    #[arg(long)]
    inject: Option<String>,
    /// Where the evidence JSON goes; raw samples go beside it as `<stem>.samples.json`.
    #[arg(long)]
    pub(super) evidence: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub(super) enum Mode {
    /// The paced program each PTY runs (started by the benchmark).
    #[command(hide = true)]
    Emit {
        #[arg(long)]
        bytes_per_second: usize,
    },
}

const fn usage(reason: &'static str) -> Terminal50Error {
    Terminal50Error::Usage { reason }
}

fn positive(value: Option<usize>, reason: &'static str) -> Result<usize, Terminal50Error> {
    value
        .filter(|value| *value > 0)
        .ok_or_else(|| usage(reason))
}

fn profile(cli: &Cli, shape: Shape) -> Result<Profile, Terminal50Error> {
    let profile = match (
        cli.admission_max_ms,
        cli.event_max_ms,
        cli.admission_p99_ms,
        cli.event_p99_ms,
    ) {
        (Some(admission_max_ms), Some(event_max_ms), None, None) => Profile::Ideal {
            admission_max_ms,
            event_max_ms,
        },
        (None, None, Some(admission_p99_ms), Some(event_p99_ms)) => Profile::Capacity {
            admission_p99_ms,
            event_p99_ms,
        },
        _ => {
            return Err(usage(
                "pass exactly one budget pair: the two max or the two p99 bounds",
            ));
        }
    };
    if matches!(profile, Profile::Ideal { .. }) && shape != IDEAL_SHAPE {
        return Err(usage(
            "the max bounds judge only the IDEAL shape: 16 terminals, 8 clients, 16 in flight, 1048576 B/s",
        ));
    }
    Ok(profile)
}

/// A private root that did not exist before this run.
fn private_root() -> PathBuf {
    let base = std::env::var_os("OVIS_FIXTURE_ROOT").map_or_else(std::env::temp_dir, PathBuf::from);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    base.join(format!("ovis-terminal50-{}-{nanos}", std::process::id()))
}

impl Cli {
    /// The validated plan and the binary under test.
    pub(super) fn plan(&self, exe: PathBuf) -> Result<(Plan, PathBuf), Terminal50Error> {
        if !self.isolated {
            return Err(Terminal50Error::NotIsolated);
        }
        let binary = self.binary.clone().ok_or_else(|| usage("pass --binary"))?;
        let shape = Shape {
            terminals: positive(self.terminals, "pass a positive --terminals")?,
            clients: positive(self.clients, "pass a positive --clients")?,
            inflight: positive(self.inflight, "pass a positive --inflight")?,
            bytes_per_second: positive(
                self.output_bytes_per_second,
                "pass a positive --output-bytes-per-second",
            )?,
        };
        let viewers = self.viewers_per_terminal.unwrap_or(shape.clients);
        if viewers < 2 {
            return Err(usage("each terminal needs a stalled and a reading viewer"));
        }
        let fixture = self
            .inject
            .as_deref()
            .map(serde_json::from_str::<Fixture>)
            .transpose()
            .map_err(|source| Terminal50Error::Inject { source })?;
        let workers = self.workers.unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map_or(2, std::num::NonZeroUsize::get)
                .clamp(2, 8)
        });
        let plan = Plan {
            shape,
            profile: profile(self, shape)?,
            samples: positive(self.samples, "pass a positive --samples")?,
            warmup: self.warmup_samples,
            viewers,
            rss_limit_bytes: self.rss_limit_mib.saturating_mul(1 << 20),
            disk_limit_bytes: self.disk_limit_mib.saturating_mul(1 << 20),
            workers: workers.max(1),
            max_run: Duration::from_secs(self.max_seconds),
            drain: Duration::from_secs(5),
            fixture,
            source: Source::Pty { exe },
            root: private_root(),
        };
        Ok((plan, binary))
    }
}
