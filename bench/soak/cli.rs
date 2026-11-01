//! The command line and its validation into one [`Plan`].

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};

use super::error::SoakError;
use super::plan::{self, Fixture, Kind, Limits, Plan};
use super::wire::BootMode;

const MIB: u64 = 1 << 20;

#[derive(Debug, Parser)]
#[command(
    name = "soak",
    version,
    about = "Run terminals continuously through control crashes, restarts, interruptions and stalled viewers.",
    args_conflicts_with_subcommands = true,
    disable_help_subcommand = true
)]
pub(super) struct Cli {
    #[command(subcommand)]
    pub(super) mode: Option<Mode>,
    /// The ovis-terminal binary under test; its holders run every program.
    #[arg(long)]
    binary: Option<PathBuf>,
    /// Run in a fresh private root; required.
    #[arg(long)]
    isolated: bool,
    #[arg(long)]
    terminals: Option<usize>,
    /// Length of the continuous measured run.
    #[arg(long)]
    duration_seconds: Option<u64>,
    /// SIGKILLs of the control process, each followed by crash recovery.
    #[arg(long, default_value_t = 0)]
    core_crashes: usize,
    #[arg(long, default_value_t = 1)]
    planned_restarts: usize,
    #[arg(long, default_value_t = 2)]
    remote_interruptions: usize,
    #[arg(long, default_value_t = 1)]
    stalled_viewers: usize,
    /// Skip the internal shared/dedicated holder-loss probe after the run.
    #[arg(long)]
    no_holder_loss_probe: bool,
    /// Defaults to a tenth of the duration, at most 180 s.
    #[arg(long)]
    warmup_seconds: Option<u64>,
    /// Baseline and final RSS window; defaults to 60 s, at most a tenth of the duration.
    #[arg(long)]
    window_seconds: Option<u64>,
    #[arg(long, default_value_t = 8192)]
    bytes_per_terminal: usize,
    #[arg(long, default_value_t = 1024)]
    rss_limit_mib: u64,
    #[arg(long, default_value_t = 4096)]
    disk_limit_mib: u64,
    #[arg(long, default_value_t = 1000)]
    control_max_ms: u64,
    /// A short smoke run of every disruption; it never counts as the acceptance run.
    #[arg(long)]
    self_test: bool,
    /// `{"fixture":"holder-tail-gap-and-lost-receipt"}`.
    #[arg(long)]
    inject: Option<String>,
    /// Report path; raw samples, checkpoints and process logs go beside it.
    #[arg(long)]
    pub(super) evidence: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub(super) enum Mode {
    /// One boot of the control process (started by the harness).
    #[command(hide = true)]
    Control {
        #[arg(long)]
        root: PathBuf,
        #[arg(long, value_enum)]
        boot_mode: BootMode,
        #[arg(long)]
        boot: u64,
    },
    /// The paced program each terminal runs (started by the holder).
    #[command(hide = true)]
    Emit {
        #[arg(long)]
        bytes_per_second: usize,
    },
}

const fn usage(reason: &'static str) -> SoakError {
    SoakError::Usage { reason }
}

/// A private root that did not exist before this run.
fn private_root() -> PathBuf {
    // Holder sockets live below this root and must fit the Unix socket path limit; the
    // macOS temporary directory alone is about half of it, so the default is `/tmp`.
    let base =
        std::env::var_os("OVIS_FIXTURE_ROOT").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_micros())
        .checked_rem(1_000_000_000)
        .unwrap_or(0);
    // Short on purpose: holder socket paths below it must stay within the Unix limit.
    base.join(format!("soak-{}-{micros}", std::process::id()))
}

impl Cli {
    fn shape(&self) -> Result<(usize, u64, &'static str), SoakError> {
        let (terminals, seconds, label) = match (self.self_test, self.fixture()?) {
            (true, _) => (
                self.terminals.unwrap_or(3),
                self.duration_seconds.unwrap_or(40),
                "self-test",
            ),
            (false, Some(_)) => (
                self.terminals.unwrap_or(2),
                self.duration_seconds.unwrap_or(0),
                "fixture",
            ),
            (false, None) => (
                self.terminals.ok_or_else(|| usage("pass --terminals"))?,
                self.duration_seconds
                    .ok_or_else(|| usage("pass --duration-seconds"))?,
                "continuous",
            ),
        };
        if terminals == 0 {
            return Err(usage("pass a positive --terminals"));
        }
        Ok((terminals, seconds, label))
    }

    fn fixture(&self) -> Result<Option<Fixture>, SoakError> {
        self.inject
            .as_deref()
            .map(serde_json::from_str::<Fixture>)
            .transpose()
            .map_err(|source| SoakError::Inject { source })
    }

    fn counts(&self) -> Vec<(Kind, usize)> {
        let ones = usize::from(self.self_test);
        let pick = |given: usize| {
            if self.self_test {
                given.max(ones)
            } else {
                given
            }
        };
        vec![
            (Kind::CoreCrash, pick(self.core_crashes)),
            (Kind::PlannedRestart, pick(self.planned_restarts)),
            (Kind::RemoteInterruption, pick(self.remote_interruptions)),
            (Kind::StalledViewer, pick(self.stalled_viewers)),
        ]
    }

    /// The validated plan and the binary under test.
    pub(super) fn plan(&self) -> Result<(Plan, PathBuf), SoakError> {
        if !self.isolated {
            return Err(SoakError::NotIsolated);
        }
        let given = self.binary.as_ref().ok_or_else(|| usage("pass --binary"))?;
        // Holders start with "/" as their working directory, so a relative path would name
        // another file there: the binary under test is pinned to its absolute path here.
        let binary = std::fs::canonicalize(given).map_err(|source| SoakError::Binary { source })?;
        let (terminals, seconds, label) = self.shape()?;
        let fixture = self.fixture()?;
        let duration = Duration::from_secs(seconds);
        let tenth = seconds.checked_div(10).unwrap_or(0);
        let warmup = Duration::from_secs(self.warmup_seconds.unwrap_or_else(|| tenth.min(180)));
        let window = Duration::from_secs(self.window_seconds.unwrap_or_else(|| tenth.min(60)));
        let counts = if fixture.is_some() {
            Vec::new()
        } else {
            self.counts()
        };
        let open = warmup.saturating_add(window);
        let close = duration.saturating_sub(window);
        let disruptions = if fixture.is_some() {
            Vec::new()
        } else {
            if duration.is_zero() || window.is_zero() {
                return Err(usage(
                    "pass a positive --duration-seconds and --window-seconds",
                ));
            }
            plan::schedule(&counts, (open, close)).ok_or_else(|| {
                usage("the disruptions do not fit between warmup plus window and the final window")
            })?
        };
        let queue_bytes = 256 * 1024;
        let plan = Plan {
            label,
            terminals,
            duration,
            warmup,
            window,
            disruptions,
            counts,
            holder_loss_probe: !self.no_holder_loss_probe && fixture.is_none() && terminals > 1,
            bytes_per_terminal: if fixture.is_some() {
                256 * 1024
            } else {
                self.bytes_per_terminal
            },
            checkpoint_every: Duration::from_secs(if self.self_test { 2 } else { 10 }),
            history_lines: 24,
            limits: Limits {
                rss_bytes: self.rss_limit_mib.saturating_mul(MIB),
                disk_bytes: self.disk_limit_mib.saturating_mul(MIB),
                control_ms: self.control_max_ms,
                queue_bytes,
                growth_bytes: 16 * MIB,
                growth_percent: 1,
            },
            fixture,
            root: private_root(),
            evidence: self.evidence.clone(),
        };
        Ok((plan, binary))
    }
}
