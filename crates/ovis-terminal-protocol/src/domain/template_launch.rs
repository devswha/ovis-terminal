//! Versioned template launches and the readiness-bound first message (plan todo 35).
//!
//! A launch names its program exactly once, either as an argument vector or as a
//! template selected by stable name and version; both go through the same launch path.
//! A first message is queued under the launch operation and the requesting principal
//! and is released only after the program declares readiness. Its state is one of
//! `accepted`, `sent`, `cancelled` or `expired`; the last three are final.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::errors::OpId;
use crate::schema::ids::{TemplateName, TerminalId};
use crate::schema::methods::template::{TemplateListResult, TemplateRecord};
use crate::schema::methods::terminal::{LaunchProgram, TerminalRunParams};
use crate::schema::records::AgentState;
use crate::schema::scalar::{DurationMs, EventSeq, Incarnation, OpenEnum, TimestampMs};

mod launched;
#[cfg(test)]
mod tests;

pub use launched::{LaunchedTerminal, started_process};

/// Readiness deadline of a first message whose launch names none.
pub const DEFAULT_READY_DEADLINE: DurationMs = DurationMs::new(30_000);

/// How a launched program proves it can take its first message.
///
/// Only a declaration by the program counts; output silence is never readiness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "by", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum ReadinessContract {
    /// The program declares one of `states` through OSC 7501 or `status.report`.
    Declared {
        /// Declared states that release the first message.
        states: Vec<OpenEnum<AgentState>>,
    },
}

impl ReadinessContract {
    /// A declared-readiness contract over `states`.
    ///
    /// # Errors
    ///
    /// [`TemplateLaunchError::UnsafeReadiness`] when `states` is empty or names a state
    /// in which typed text could answer a dialog or interrupt a turn.
    pub fn declared(states: Vec<AgentState>) -> Result<Self, TemplateLaunchError> {
        Self::Declared {
            states: states.into_iter().map(OpenEnum::Known).collect(),
        }
        .validated()
    }

    /// The default contract: the program declares `idle`.
    #[must_use]
    pub fn idle() -> Self {
        Self::Declared {
            states: vec![OpenEnum::Known(AgentState::Idle)],
        }
    }

    /// Declared states that release the first message; unknown states never do.
    #[must_use]
    pub fn states(&self) -> Vec<AgentState> {
        match self {
            Self::Declared { states } => states
                .iter()
                .filter_map(|state| match state {
                    OpenEnum::Known(state) => Some(*state),
                    OpenEnum::Unknown(_) => None,
                })
                .collect(),
        }
    }

    /// Rechecks a contract read from an untrusted definition.
    ///
    /// # Errors
    ///
    /// [`TemplateLaunchError::UnsafeReadiness`] when it names no state, an unknown state,
    /// or a state in which typed text could answer a dialog or interrupt a turn.
    pub fn validated(self) -> Result<Self, TemplateLaunchError> {
        let Self::Declared { states } = &self;
        let safe = |state: &OpenEnum<AgentState>| {
            matches!(state, OpenEnum::Known(AgentState::Idle | AgentState::Done))
        };
        if states.is_empty() || !states.iter().all(safe) {
            return Err(TemplateLaunchError::UnsafeReadiness);
        }
        Ok(self)
    }
}

impl TemplateRecord {
    /// A template launching `argv` with no environment, `idle` readiness and no body.
    #[must_use]
    pub fn new(
        (name, version): (TemplateName, String),
        description: String,
        argv: Vec<String>,
    ) -> Self {
        Self {
            name,
            version,
            description,
            argv,
            env: BTreeMap::new(),
            readiness: ReadinessContract::idle(),
            ready_deadline_ms: DEFAULT_READY_DEADLINE,
            body: None,
        }
    }
}

impl TemplateListResult {
    /// The listing of `templates`.
    #[must_use]
    pub const fn new(templates: Vec<TemplateRecord>) -> Self {
        Self { templates }
    }
}

/// Where a first message is in its life; `sent`, `cancelled` and `expired` are final.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FirstMessageState {
    /// Queued under the launch; the program has not declared readiness.
    Accepted,
    /// Handed to the terminal's writer exactly once after declared readiness.
    Sent,
    /// Never written: the program ended, or a write-boundary recheck refused it.
    Cancelled,
    /// Never written: readiness was not declared before the deadline.
    Expired,
}

/// Why a first message reached a final state other than `sent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FirstMessageReason {
    /// The deadline passed before the program declared readiness.
    DeadlinePassed,
    /// The program exited before declaring readiness.
    ProcessExited,
    /// The holder link ended before readiness.
    LinkLost,
    /// Another program incarnation runs in the terminal now.
    ProcessReplaced,
    /// The requesting principal's credential lapsed before the write.
    AuthorityLapsed,
    /// An interact lease is held, so an unleased write is refused.
    LeaseHeld,
    /// The text cannot be encoded for the terminal's current modes.
    EncodingRefused,
    /// The terminal's writer refused or failed the write.
    DeliveryFailed,
    /// The runtime stopped waiting before readiness.
    RuntimeStopped,
}

/// The first message queued under one launch, readable by its requesting principal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct FirstMessageRecord {
    /// Operation that launched the terminal; the message's identity.
    pub launch_op: OpId,
    /// Terminal the message is bound to.
    pub terminal_id: TerminalId,
    /// Program incarnation the message is bound to.
    pub process_incarnation: Incarnation,
    /// Current state.
    pub state: OpenEnum<FirstMessageState>,
    /// Instant after which an unreleased message expires.
    pub deadline_ms: TimestampMs,
    /// Input sequence of the single submission, once sent.
    pub queued_seq: Option<EventSeq>,
    /// Why the message was not sent, once cancelled or expired.
    pub reason: Option<OpenEnum<FirstMessageReason>>,
}

impl FirstMessageRecord {
    /// A message accepted under `launch_op` for `terminal_id` at `process_incarnation`.
    #[must_use]
    pub fn accepted(
        (launch_op, terminal_id): (OpId, TerminalId),
        process_incarnation: Incarnation,
        deadline_ms: TimestampMs,
    ) -> Self {
        Self {
            launch_op,
            terminal_id,
            process_incarnation,
            state: OpenEnum::Known(FirstMessageState::Accepted),
            deadline_ms,
            queued_seq: None,
            reason: None,
        }
    }

    /// Whether the message is still waiting for readiness.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        matches!(self.state, OpenEnum::Known(FirstMessageState::Accepted))
    }
}

/// The program a launch names, exactly once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LaunchSource<'a> {
    /// A literal argument vector.
    Argv(&'a [String]),
    /// A template by stable name and optional version, with task text as data.
    Template {
        /// Template name.
        name: &'a TemplateName,
        /// Pinned version; absent selects the latest.
        version: Option<&'a str>,
        /// Task text; never interpolated into a command line.
        task: Option<&'a str>,
    },
}

impl<'a> LaunchSource<'a> {
    /// The single program `params` names.
    ///
    /// # Errors
    ///
    /// [`TemplateLaunchError::ProgramMissing`] when neither `program` nor `template` is
    /// present, [`TemplateLaunchError::ProgramConflict`] when both are.
    pub fn of(params: &'a TerminalRunParams) -> Result<Self, TemplateLaunchError> {
        match (&params.program, &params.template) {
            (Some(LaunchProgram::Argv(argv)), None) => Ok(Self::Argv(argv)),
            (
                Some(LaunchProgram::Template {
                    name,
                    version,
                    task,
                }),
                None,
            ) => Ok(Self::Template {
                name,
                version: version.as_deref(),
                task: task.as_deref(),
            }),
            (None, Some(selector)) => Ok(Self::Template {
                name: &selector.name,
                version: selector.version.as_deref(),
                task: selector.task.as_deref(),
            }),
            (None, None) => Err(TemplateLaunchError::ProgramMissing),
            (Some(_), Some(_)) => Err(TemplateLaunchError::ProgramConflict),
        }
    }
}

/// A launch request or template definition the launch contract refuses.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TemplateLaunchError {
    /// Neither an argument vector nor a template was named.
    #[error("the launch names no program")]
    ProgramMissing,
    /// Both an argument vector and a template were named.
    #[error("the launch names a program and a template at once")]
    ProgramConflict,
    /// The readiness contract names no state, or a state that is not safe for typing.
    #[error("the readiness contract must declare idle or done states only")]
    UnsafeReadiness,
}

impl TemplateLaunchError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::ProgramMissing | Self::ProgramConflict => "invalid_params",
            Self::UnsafeReadiness => "template_invalid",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::ProgramMissing => "name the program as program.argv or as template",
            Self::ProgramConflict => "send either program or template, not both",
            Self::UnsafeReadiness => {
                "declare readiness by idle or done; output silence is never readiness"
            }
        }
    }

    /// Request field the refusal is about, when it is a request field.
    #[must_use]
    pub const fn field_path(&self) -> Option<&'static str> {
        match self {
            Self::ProgramMissing => Some("params.program"),
            Self::ProgramConflict => Some("params.template"),
            Self::UnsafeReadiness => None,
        }
    }
}
