//! Resident memory of the harness and the programs it drives.

use ovis_terminal_probes::platform::{ByteCount, Pid, ProbeError, resident_memory};
use serde::Serialize;

/// Resident memory at one sample point.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ResidentMemory {
    /// The harness process, which owns every pseudo-terminal master.
    pub harness: ByteCount,
    /// Sum over the programs running in the terminals.
    pub programs: ByteCount,
}

impl ResidentMemory {
    pub(super) fn sample(programs: &[Pid]) -> Result<Self, ProbeError> {
        let harness = resident_memory(Pid::new(std::process::id()))?;
        let mut total = ByteCount::default();
        for pid in programs {
            total = total.saturating_add(resident_memory(*pid)?.get());
        }
        Ok(Self {
            harness,
            programs: total,
        })
    }

    /// Harness plus programs.
    #[must_use]
    pub const fn total(self) -> ByteCount {
        self.harness.saturating_add(self.programs.get())
    }
}
