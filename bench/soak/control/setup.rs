//! What the harness tells every control boot, and what the first boot records durably.

use std::path::{Path, PathBuf};

use ovis_terminal_core::recovery::{GroupId, HolderGroup, Recorded};
use ovis_terminal_holder::protocol::{ProcessStartToken, TerminalId, TerminalTarget};
use ovis_terminal_protocol::schema::ids::ProcessPid;
use ovis_terminal_protocol::schema::scalar::{ByteCount, Count};
use ovis_terminal_vt::{Geometry, HistoryLimit, VtConfig};
use serde::{Deserialize, Serialize};

use super::super::error::SoakError;

const SETUP: &str = "setup.json";
const RECORDED: &str = "recorded.json";

/// The build every soak holder group is named after.
const BUILD: &str = "soak";

/// Where a holder group runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in super::super) enum Placement {
    Shared,
    /// One terminal alone in its own holder: the internal isolation placement.
    Dedicated,
}

/// One holder process of the binary under test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in super::super) struct GroupSpec {
    pub(in super::super) placement: Placement,
    pub(in super::super) session: String,
    pub(in super::super) socket: PathBuf,
}

/// The run as every control boot reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in super::super) struct Setup {
    /// Shared group first; a dedicated group, when present, holds the last terminal.
    pub(in super::super) groups: Vec<GroupSpec>,
    pub(in super::super) terminals: Count,
    /// argv of every program, run by the holder with an empty environment but `PATH`.
    pub(in super::super) program: Vec<String>,
    pub(in super::super) state: PathBuf,
    pub(in super::super) socket: PathBuf,
    pub(in super::super) checkpoint_every_ms: Count,
    pub(in super::super) history_lines: Count,
    pub(in super::super) queue_limit_bytes: ByteCount,
    /// The holder-tail-gap-and-lost-receipt fixture: checkpoint terminal 0 once only.
    pub(in super::super) fixture: bool,
}

/// One program the first boot started, as later boots read it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub(super) target: TerminalTarget,
    pub(super) group: Count,
    pub(super) pid: ProcessPid,
    pub(super) start_token: ProcessStartToken,
}

pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), SoakError> {
    let staged = path.with_extension("staged");
    std::fs::write(&staged, bytes).map_err(SoakError::io("write a staged record"))?;
    std::fs::File::open(&staged)
        .and_then(|file| file.sync_all())
        .map_err(SoakError::io("sync a staged record"))?;
    std::fs::rename(&staged, path).map_err(SoakError::io("publish a record"))
}

pub(super) fn read<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, SoakError> {
    let bytes = std::fs::read(path).map_err(SoakError::io("read a control record"))?;
    Ok(serde_json::from_slice(&bytes)?)
}

impl Setup {
    pub(in super::super) fn save(&self, root: &Path) -> Result<(), SoakError> {
        write_atomic(&root.join(SETUP), &serde_json::to_vec_pretty(self)?)
    }

    pub(in super::super) fn load(root: &Path) -> Result<Self, SoakError> {
        read(&root.join(SETUP))
    }

    pub(super) fn config(&self) -> Result<VtConfig, SoakError> {
        let geometry = Geometry::new(80, 24).map_err(|_| SoakError::Usage {
            reason: "the fixed 80x24 geometry was rejected",
        })?;
        let lines = u32::try_from(self.history_lines.get()).unwrap_or(u32::MAX);
        Ok(VtConfig::new(geometry, HistoryLimit::new(lines)))
    }

    pub(super) fn count(&self) -> usize {
        usize::try_from(self.terminals.get()).unwrap_or(usize::MAX)
    }

    /// The group index terminal `index` lives in.
    pub(super) fn group_of(&self, index: usize) -> usize {
        let dedicated = self
            .groups
            .iter()
            .position(|group| group.placement == Placement::Dedicated);
        match dedicated {
            Some(group) if index.saturating_add(1) == self.count() && index > 0 => group,
            _ => 0,
        }
    }

    pub(super) fn terminal_id(index: usize) -> TerminalId {
        TerminalId::new(format!("terminal:soak-{index}"))
    }

    /// The recovery identity of group `group`.
    pub(super) fn group_id(&self, group: usize) -> GroupId {
        match self.groups.get(group).map(|spec| spec.placement) {
            Some(Placement::Dedicated) => {
                GroupId::dedicated(&Self::terminal_id(self.count().saturating_sub(1)))
            }
            _ => GroupId::shared(
                self.groups
                    .first()
                    .map_or("soak", |spec| spec.session.as_str()),
                BUILD,
            ),
        }
    }

    pub(super) fn holder_groups(&self) -> Vec<HolderGroup> {
        self.groups
            .iter()
            .enumerate()
            .map(|(index, spec)| HolderGroup::new(self.group_id(index), spec.socket.clone()))
            .collect()
    }

    pub(super) fn save_recorded(&self, entries: &[Entry]) -> Result<(), SoakError> {
        write_atomic(
            &self.state.join(RECORDED),
            &serde_json::to_vec_pretty(entries)?,
        )
    }

    pub(super) fn recorded(&self) -> Result<Vec<Entry>, SoakError> {
        read(&self.state.join(RECORDED))
    }

    /// What recovery is told was running before the boundary.
    pub(super) fn as_recorded(&self, entry: &Entry) -> Recorded {
        let group = usize::try_from(entry.group.get()).unwrap_or(usize::MAX);
        Recorded::new(
            entry.target.clone(),
            (entry.pid.get(), entry.start_token.clone()),
            self.group_id(group),
        )
    }
}
