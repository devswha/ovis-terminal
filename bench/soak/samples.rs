//! Raw samples and checkpoints, kept durably beside the evidence while the run goes on:
//! one JSON line per second, flushed as it is taken, and a checkpoint file replaced
//! atomically, so an interrupted run still leaves everything it measured.

use std::fs::File;
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ovis_terminal_probes::platform::{Pid, resident_memory};
use ovis_terminal_protocol::schema::scalar::{ByteCount, Count, TimestampMs};
use serde::Serialize;

use super::error::SoakError;

/// One sample of the runtime's footprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct Sample {
    pub(super) elapsed_ms: Count,
    pub(super) boot: Count,
    /// Control process plus every holder: the runtime the 1 GiB budget covers.
    pub(super) runtime_rss: ByteCount,
    pub(super) control_rss: ByteCount,
    pub(super) holder_rss: ByteCount,
    /// The programs in the terminals, reported apart from the runtime.
    pub(super) program_rss: ByteCount,
    pub(super) disk_bytes: ByteCount,
}

/// Resident bytes of `pids`; a PID that is gone counts zero and is reported.
pub(super) fn rss(pids: &[u32]) -> (u64, u64) {
    pids.iter().fold((0, 0), |(total, missing), pid| {
        resident_memory(Pid::new(*pid)).map_or_else(
            |_| (total, missing.saturating_add(1)),
            |bytes| (total.saturating_add(bytes.get()), missing),
        )
    })
}

#[derive(Debug)]
pub(super) struct Samples {
    file: BufWriter<File>,
    pub(super) path: PathBuf,
    pub(super) checkpoint: PathBuf,
    pub(super) started_unix_ms: TimestampMs,
    /// `(elapsed ms, runtime rss)` of every sample.
    pub(super) series: Vec<(u64, u64)>,
    pub(super) peak_runtime: u64,
    pub(super) peak_disk: u64,
    pub(super) count: u64,
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

impl Samples {
    /// Opens `<stem>.samples.ndjson` and `<stem>.checkpoint.json` beside `evidence`.
    pub(super) fn open(evidence: &Path) -> Result<Self, SoakError> {
        if let Some(parent) = evidence
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(SoakError::io("create the evidence directory"))?;
        }
        let path = evidence.with_extension("samples.ndjson");
        let file = File::create(&path).map_err(SoakError::io("create the samples file"))?;
        Ok(Self {
            file: BufWriter::new(file),
            checkpoint: evidence.with_extension("checkpoint.json"),
            path,
            started_unix_ms: TimestampMs::new(unix_ms()),
            series: Vec::new(),
            peak_runtime: 0,
            peak_disk: 0,
            count: 0,
        })
    }

    pub(super) fn record(&mut self, sample: &Sample) -> Result<(), SoakError> {
        let mut line = serde_json::to_vec(sample)?;
        line.push(b'\n');
        self.file
            .write_all(&line)
            .and_then(|()| self.file.flush())
            .map_err(SoakError::io("append a sample"))?;
        let runtime = sample.runtime_rss.get();
        self.series.push((sample.elapsed_ms.get(), runtime));
        self.peak_runtime = self.peak_runtime.max(runtime);
        self.peak_disk = self.peak_disk.max(sample.disk_bytes.get());
        self.count = self.count.saturating_add(1);
        Ok(())
    }

    /// Syncs the samples and atomically replaces the checkpoint with `snapshot`.
    pub(super) fn checkpoint(&mut self, snapshot: &impl Serialize) -> Result<(), SoakError> {
        self.file
            .flush()
            .map_err(SoakError::io("flush the samples"))?;
        self.file
            .get_ref()
            .sync_data()
            .map_err(SoakError::io("sync the samples"))?;
        let staged = self.checkpoint.with_extension("staged");
        std::fs::write(&staged, serde_json::to_vec_pretty(snapshot)?)
            .map_err(SoakError::io("write a checkpoint"))?;
        std::fs::rename(&staged, &self.checkpoint).map_err(SoakError::io("publish a checkpoint"))
    }

    /// Median runtime RSS of the samples taken in `[from, to)`.
    pub(super) fn median(&self, (from, to): (Duration, Duration)) -> Option<u64> {
        median_in(&self.series, (from, to))
    }
}

/// Median of the values whose elapsed time lies in `[from, to)`.
pub(super) fn median_in(series: &[(u64, u64)], (from, to): (Duration, Duration)) -> Option<u64> {
    let from = u64::try_from(from.as_millis()).unwrap_or(u64::MAX);
    let to = u64::try_from(to.as_millis()).unwrap_or(u64::MAX);
    let mut values: Vec<u64> = series
        .iter()
        .filter(|(at, _)| *at >= from && *at < to)
        .map(|(_, value)| *value)
        .collect();
    values.sort_unstable();
    values.get(values.len().checked_div(2)?).copied()
}
