//! Process roles are native evidence or explicitly unavailable, never argv guesses.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::registry::{ProcessHandle, TerminalId};

/// Why a requested process field could not be observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Unavailable {
    /// This native adapter does not expose the field.
    Platform,
    /// No authoritative shell role was declared for an explicit executable launch.
    UndeclaredRole,
    /// The process no longer has a readable native record.
    ProcessUnavailable,
    /// Native process evidence changed during observation.
    Raced,
    /// Native permissions prevented inspection.
    Permission,
    /// Native strings cannot be represented as UTF-8.
    Encoding,
    /// The process table exceeded the bounded inspection budget.
    Limit,
}

/// Explicit field availability; absent evidence is never evidence of exit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "value", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Field<T> {
    /// The field was observed.
    Known(T),
    /// The field could not be observed for the stated reason.
    Unsupported(Unavailable),
}

/// One native process observation, bound by PID and kernel start token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ProcessSample {
    /// PID and reuse discriminator observed together.
    pub process: ProcessHandle,
    /// Current kernel argument vector, not an agent-brand guess.
    pub argv: Field<Vec<String>>,
    /// Current working directory.
    pub cwd: Field<PathBuf>,
}

/// Structured launcher, shell, foreground and background evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ProcessInfo {
    /// Exact terminal whose PTY was inspected.
    pub terminal: TerminalId,
    /// A shell role is not inferred from process names.
    pub shell: Field<ProcessSample>,
    /// The program actually launched by the holder.
    pub launcher: Field<ProcessSample>,
    /// Native foreground process group members.
    pub foreground: Field<Vec<ProcessSample>>,
    /// Other processes in the same controlling-terminal session.
    pub background: Field<Vec<ProcessSample>>,
}
