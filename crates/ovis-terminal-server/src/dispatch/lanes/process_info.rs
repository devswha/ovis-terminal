//! `terminal.process_info` (plan todo 55): roles of exactly the named terminal.
//!
//! The target is explicit and closed: a misnamed field is `invalid_params` before any
//! process is read, and nothing ever falls back to a focused terminal (EXCL-006). A
//! process proven replaced or exited by its start token is reported `process_replaced`
//! and its ownership is revoked through the directory; a field the operating system
//! cannot prove is an unsupported value, never an exit.

use std::fmt::Debug;
use std::sync::Arc;

use ovis_terminal_authz::{Action, Session};
use ovis_terminal_core::registry::{ProcessHandle, TargetResolutionError, TerminalId};
use ovis_terminal_core::terminals::process_info::{self, Liveness, ProcessTable};
use ovis_terminal_protocol::domain::process_info::{ProcessRoles, UnsupportedReason};
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::schema::ids::TerminalId as WireTerminalId;
use ovis_terminal_protocol::schema::methods::terminal::{
    TerminalProcessInfoParams, TerminalProcessInfoResult,
};
use ovis_terminal_protocol::schema::records::ProcessRecord;
use ovis_terminal_protocol::schema::scalar::Incarnation;
use ovis_terminal_store::Snapshot;
use serde_json::Value;

use super::agent_status::{ResolvedTerminal, TerminalDirectory, terminal_access};
use crate::dispatch::{Access, DispatchError, Read};

/// The program a terminal currently runs, as its owner recorded it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Launched {
    /// Launch facts, including the program incarnation.
    pub record: ProcessRecord,
    /// PID and start token while the holder has not recorded an exit.
    pub owned: Option<ProcessHandle>,
}

impl Launched {
    /// A program described by `record`, owned as `owned` until its exit is recorded.
    #[must_use]
    pub const fn new(record: ProcessRecord, owned: Option<ProcessHandle>) -> Self {
        Self { record, owned }
    }
}

/// A program incarnation the process table proved replaced or exited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Ended {
    /// Terminal that ran it.
    pub terminal: TerminalId,
    /// Its program incarnation.
    pub incarnation: Incarnation,
    /// [`Liveness::Replaced`] or [`Liveness::Exited`].
    pub proof: Liveness,
}

/// The owner of terminal programs: resolves targets and revokes ended incarnations.
pub trait ProcessDirectory: TerminalDirectory {
    /// The current program of `terminal`.
    ///
    /// # Errors
    ///
    /// Returns `not_found` when the terminal is gone.
    fn launched(&self, terminal: TerminalId) -> Result<Launched, TargetResolutionError>;

    /// Revokes agent and session ownership bound to an ended incarnation.
    fn ended(&self, ended: Ended);
}

impl<T: ProcessDirectory> ProcessDirectory for Arc<T> {
    fn launched(&self, terminal: TerminalId) -> Result<Launched, TargetResolutionError> {
        T::launched(self, terminal)
    }

    fn ended(&self, ended: Ended) {
        T::ended(self, ended);
    }
}

/// Why `terminal.process_info` reported no roles.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProcessInfoError {
    /// The explicit target did not resolve.
    #[error("the explicit target could not be resolved")]
    Target(#[from] TargetResolutionError),
    /// The terminal is not the expected terminal incarnation.
    #[error("the terminal incarnation differs from the expected one")]
    StaleTarget,
    /// The expected program incarnation no longer runs.
    #[error("the expected program no longer runs in this terminal")]
    ProcessReplaced,
}

impl ProcessInfoError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Target(error) => error.kind(),
            Self::StaleTarget => "stale_target",
            Self::ProcessReplaced => "process_replaced",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Target(error) => error.hint(),
            Self::StaleTarget => "read the terminal again and target its current incarnation",
            Self::ProcessReplaced => {
                "read terminal.process_info without expect; never redirect to the new program"
            }
        }
    }

    fn boundary(self) -> BoundaryError {
        match self {
            Self::Target(error) => DispatchError::from(error).boundary(),
            Self::StaleTarget | Self::ProcessReplaced => {
                BoundaryError::domain(ErrorKind::from(self.kind().to_owned()), false)
                    .with_hint(self.hint())
            }
        }
    }
}

/// `terminal.process_info` over a directory of terminals and a process table.
#[derive(Debug)]
pub struct ProcessInfoRead<T, S> {
    directory: T,
    table: Arc<S>,
}

impl<T: ProcessDirectory, S: ProcessTable + Debug + Send + Sync + 'static> ProcessInfoRead<T, S> {
    /// Answers for the terminals `directory` owns, reading processes from `table`.
    pub const fn new(directory: T, table: Arc<S>) -> Self {
        Self { directory, table }
    }

    fn checked(
        &self,
        params: &TerminalProcessInfoParams,
    ) -> Result<(ResolvedTerminal, Launched), ProcessInfoError> {
        let resolved = self.directory.resolve(&params.target)?;
        let launched = self.directory.launched(resolved.terminal)?;
        let expect = params.expect.clone().unwrap_or_default();
        if expect
            .terminal_incarnation
            .is_some_and(|expected| expected.get() != resolved.generation.get())
        {
            return Err(ProcessInfoError::StaleTarget);
        }
        if expect
            .process_incarnation
            .is_some_and(|expected| expected != launched.record.incarnation)
        {
            return Err(ProcessInfoError::ProcessReplaced);
        }
        Ok((resolved, launched))
    }

    /// The roles of the named terminal, or why there are none.
    ///
    /// # Errors
    ///
    /// Returns a target failure, `stale_target`, or `process_replaced` when the expected
    /// or the owned program is proven no longer to run.
    pub fn answer(
        &self,
        params: &TerminalProcessInfoParams,
    ) -> Result<TerminalProcessInfoResult, ProcessInfoError> {
        let (resolved, launched) = self.checked(params)?;
        let roles = match &launched.owned {
            None => ProcessRoles::unsupported(UnsupportedReason::ProcessUnavailable),
            Some(owned) => match process_info::observe(&*self.table, resolved.terminal, owned) {
                Ok(info) => info.roles(),
                Err(proof) => {
                    self.directory.ended(Ended {
                        terminal: resolved.terminal,
                        incarnation: launched.record.incarnation,
                        proof,
                    });
                    return Err(ProcessInfoError::ProcessReplaced);
                }
            },
        };
        Ok(TerminalProcessInfoResult::new(
            WireTerminalId::new(resolved.terminal.to_string()),
            launched.record,
            roles,
        ))
    }
}

impl<T, S> Read for ProcessInfoRead<T, S>
where
    T: ProcessDirectory,
    S: ProcessTable + Debug + Send + Sync + 'static,
{
    const NAME: &'static str = "terminal.process_info";
    type Params = TerminalProcessInfoParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        terminal_access(&self.directory, &params.target, Action::Read)
    }

    fn read(&self, params: Self::Params, _: &Snapshot) -> Result<Value, BoundaryError> {
        let result = self.answer(&params).map_err(ProcessInfoError::boundary)?;
        serde_json::to_value(result).map_err(|error| DispatchError::from(error).boundary())
    }
}

#[cfg(test)]
mod tests;
