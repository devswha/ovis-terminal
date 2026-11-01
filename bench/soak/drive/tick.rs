//! The per-second tick of a run: one footprint sample, one control ping, and every fifth
//! tick a status read on the observer and on each stalled viewer.

use std::time::{Duration, Instant};

use ovis_terminal_protocol::schema::scalar::{ByteCount, Count};

use super::super::error::SoakError;
use super::super::holders::disk_bytes;
use super::super::launch::ControlProcess;
use super::super::samples::{Sample, rss};
use super::super::wire::Request;
use super::Run;

/// Samples between two durable checkpoints.
const CHECKPOINT_EVERY: u64 = 60;

impl Run<'_> {
    /// One sample, one control ping and, every fifth tick, a status read.
    pub(in super::super) fn tick(&mut self, elapsed: Duration) -> Result<(), SoakError> {
        let control = self.control.as_ref().map_or(0, ControlProcess::pid);
        let holders: Vec<u32> = self.holders.iter().map(|holder| holder.pid).collect();
        let (control_rss, _) = rss(&[control]);
        let (holder_rss, _) = rss(&holders);
        let (program_rss, _) = rss(&self.programs);
        let sample = Sample {
            elapsed_ms: Count::new(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)),
            boot: Count::new(self.boot),
            runtime_rss: ByteCount::new(control_rss.saturating_add(holder_rss)),
            control_rss: ByteCount::new(control_rss),
            holder_rss: ByteCount::new(holder_rss),
            program_rss: ByteCount::new(program_rss),
            disk_bytes: ByteCount::new(disk_bytes(&self.plan.root)?),
        };
        self.samples.record(&sample)?;
        if self.ping.take().is_some() {
            self.tally.late();
        }
        self.pings = self.pings.saturating_add(1);
        let id = self.pings;
        self.observer()?
            .send(Request::Ping { id: Count::new(id) })?;
        self.ping = Some((id, Instant::now()));
        if id.is_multiple_of(5) {
            self.observer()?
                .send(Request::Status { id: Count::new(id) })?;
            // A stalled viewer asks without reading, so its own queue grows; once its
            // boot cut or ended it, the write fails and it leaves the set.
            let asked = format!(
                "{}\n",
                serde_json::json!({"status": {"id": id.to_string()}})
            );
            self.stalled
                .retain_mut(|stalled| std::io::Write::write_all(stalled, asked.as_bytes()).is_ok());
        }
        if self.samples.count.is_multiple_of(CHECKPOINT_EVERY) {
            let snapshot = serde_json::json!({
                "started_unix_ms": self.samples.started_unix_ms,
                "elapsed_ms": sample.elapsed_ms,
                "boot": sample.boot,
                "done": self.done,
                "tally": self.tally,
                "continuity": self.continuity,
            });
            self.samples.checkpoint(&snapshot)?;
        }
        Ok(())
    }
}
