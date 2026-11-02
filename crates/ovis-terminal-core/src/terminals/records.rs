//! Durable terminal facts; admission, process evidence and metadata are separate.

use std::collections::BTreeMap;
use std::path::PathBuf;

use ovis_terminal_holder::protocol::{Exit, Hangup, SpawnFailed, TerminalTarget};
use ovis_terminal_protocol::schema::common::Geometry;
use ovis_terminal_protocol::schema::scalar::{ByteOffset, Revision};
use serde::{Deserialize, Serialize};

use super::TerminalError;
use crate::registry::{ResourceName, TerminalId, TerminalLifecycle};

/// Explicit process inputs, with no implicit shell or inherited environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawProgram")]
pub struct Program {
    /// Executable followed by literal arguments.
    pub(super) argv: Vec<String>,
    /// Absolute working directory.
    pub(super) cwd: PathBuf,
    /// Complete environment.
    pub(super) env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProgram {
    argv: Vec<String>,
    cwd: PathBuf,
    env: BTreeMap<String, String>,
}

impl TryFrom<RawProgram> for Program {
    type Error = TerminalError;

    fn try_from(raw: RawProgram) -> Result<Self, Self::Error> {
        Self::new(raw.argv, raw.cwd, raw.env)
    }
}

impl Program {
    /// Parses the explicit launch boundary.
    ///
    /// # Errors
    ///
    /// Returns `invalid_params` for empty argv, relative cwd, NUL bytes or invalid environment names.
    pub fn new(
        argv: Vec<String>,
        cwd: PathBuf,
        env: BTreeMap<String, String>,
    ) -> Result<Self, TerminalError> {
        if argv.first().is_none_or(String::is_empty)
            || argv.iter().any(|arg| arg.contains('\0'))
            || !cwd.is_absolute()
            || cwd.to_str().is_none_or(|path| path.contains('\0'))
            || env.iter().any(|(key, value)| {
                key.is_empty() || key.contains(['=', '\0']) || value.contains('\0')
            })
        {
            return Err(TerminalError::InvalidParams);
        }
        Ok(Self { argv, cwd, env })
    }
}

/// A process launch and the metadata of its terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Launch {
    /// Literal program inputs.
    pub program: Program,
    /// Initial columns and rows.
    pub geometry: Geometry,
    /// Optional unique live name.
    pub name: Option<ResourceName>,
    /// User metadata, never process status.
    pub labels: BTreeMap<String, String>,
    /// Whether an ended process may be explicitly recovered.
    pub keep: bool,
}

impl Launch {
    /// Leaves room for process evidence inside the operation ledger's 16 KiB receipt bound.
    pub const MAX_ENCODED_BYTES: usize = 8192;

    /// Decodes a closed launch boundary without silently ignoring targeting fields.
    ///
    /// # Errors
    ///
    /// Returns `invalid_params` with the parser source for malformed fields.
    pub fn decode(bytes: &[u8]) -> Result<Self, TerminalError> {
        let launch: Self = serde_json::from_slice(bytes).map_err(TerminalError::InvalidJson)?;
        launch.validate()?;
        Ok(launch)
    }

    /// Validates geometry without launching anything.
    ///
    /// # Errors
    ///
    /// Returns `invalid_params` for zero dimensions.
    pub fn new(program: Program, geometry: Geometry) -> Result<Self, TerminalError> {
        let launch = Self {
            program,
            geometry,
            name: None,
            labels: BTreeMap::new(),
            keep: false,
        };
        launch.validate()?;
        Ok(launch)
    }

    pub(super) fn validate(&self) -> Result<(), TerminalError> {
        if self.geometry.cols == 0
            || self.geometry.rows == 0
            || self
                .labels
                .keys()
                .any(|key| key.is_empty() || key.contains('='))
        {
            return Err(TerminalError::InvalidParams);
        }
        if serde_json::to_vec(self)?.len() > Self::MAX_ENCODED_BYTES {
            return Err(TerminalError::LimitExceeded);
        }
        Ok(())
    }
}

/// Launch admission is not evidence that a process exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LaunchState {
    /// A terminal resource was admitted; no holder effect has begun.
    Admitted,
    /// The holder request is outstanding.
    Pending,
    /// PID-plus-start-token spawn evidence was received.
    Spawned,
    /// The holder proved that no process could be spawned.
    Failed,
}

/// Persisted terminal state shared by queries, lifecycle events and operation outcomes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Record {
    /// Whether control is connected, stopped, or handing ownership to a new core.
    pub control: super::ControlState,
    /// Operation that admitted the most recent launch attempt.
    pub launch_op: ovis_terminal_protocol::errors::OpId,
    /// Registry-owned terminal and process identities and transition history.
    pub lifecycle: TerminalLifecycle,
    /// Explicit launch inputs and editable metadata.
    pub launch: Launch,
    /// Independently inspectable launch outcome.
    pub spawn: LaunchState,
    /// Holder identity tuple for the actual owned process.
    pub holder_target: Option<TerminalTarget>,
    /// EOF evidence, not an exit-status claim.
    pub hangup: Option<Hangup>,
    /// Final holder evidence, including unknown exit or I/O failure.
    pub exit: Option<Exit>,
    /// Typed spawn rejection, never a synthetic live process.
    pub failure: Option<SpawnFailed>,
    /// A pre-spawn authority rejection or an explicitly uncertain worker failure.
    pub rejection: Option<ovis_terminal_protocol::errors::ErrorData>,
    /// Last native process binding; agent actions revalidate its PID and start token.
    pub agent_owner: Option<crate::registry::ProcessHandle>,
    /// Raw bytes observed through the holder's ordered stream.
    pub output_offset: ByteOffset,
    /// Durable store revision, stamped when the snapshot reads the row.
    pub revision: Revision,
    /// Creation, relaunch and metadata-only restoration never imply the same survival.
    pub origin: ovis_terminal_protocol::schema::records::TerminalOrigin,
}

impl Record {
    /// Stable terminal identity.
    pub const fn id(&self) -> TerminalId {
        self.lifecycle.terminal_id()
    }

    /// Restores metadata only after independent host/runtime reboot evidence.
    ///
    /// # Errors
    ///
    /// Returns the registry's typed rejection unless host, runtime and control boot changed.
    pub fn restore_metadata(
        mut self,
        identity: crate::registry::RuntimeIdentity,
    ) -> Result<Self, TerminalError> {
        self.lifecycle = self
            .lifecycle
            .transition(crate::registry::LifecycleEvent::HostReboot { identity })?;
        self.origin = ovis_terminal_protocol::schema::records::TerminalOrigin::RestoredMetadata;
        self.control = super::ControlState::Stopped;
        self.holder_target = None;
        self.agent_owner = None;
        self.hangup = None;
        self.exit = None;
        Ok(self)
    }
}
