//! Measured footprints: resident memory of the harness and its programs, history on disk.

use std::path::Path;

use ovis_terminal_probes::platform::{ByteCount, Pid, resident_memory};

use super::error::TuningError;

pub(super) fn rss(pids: &[Pid]) -> Result<u64, TuningError> {
    let mut total = resident_memory(Pid::new(std::process::id()))?.get();
    for pid in pids {
        total = total.saturating_add(resident_memory(*pid).map_or(0, ByteCount::get));
    }
    Ok(total)
}

/// Bytes of every file below `root`, read from file metadata.
pub(super) fn disk_bytes(root: &Path) -> Result<u64, TuningError> {
    let mut total = 0_u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    Ok(total)
}
