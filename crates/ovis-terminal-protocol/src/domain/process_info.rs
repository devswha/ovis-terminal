//! Process roles of one terminal, each bound by PID plus start token (plan todo 55).
//!
//! Every role is either observed or explicitly unsupported with a reason; an
//! unsupported foreground is never evidence that a program exited.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::schema::ids::{ProcessPid, StartToken, TerminalId};
use crate::schema::methods::terminal::TerminalProcessInfoResult;
use crate::schema::records::ProcessRecord;
use crate::schema::scalar::OpenEnum;

/// Why a process field is not reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum UnsupportedReason {
    /// The operating system does not expose the field to the runtime.
    Platform,
    /// The launch did not declare this role; it is never guessed from a name.
    UndeclaredRole,
    /// The process has no readable native record.
    ProcessUnavailable,
    /// Native evidence changed while it was read.
    Raced,
    /// Native permissions refused the read.
    Permission,
    /// The native value is not representable as UTF-8 or has an unknown layout.
    Encoding,
    /// The process table exceeded the bounded inspection budget.
    Limit,
}

/// One process field: observed, or not reported for a stated reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
#[schemars(rename = "Observed_{T}")]
#[non_exhaustive]
pub enum Observed<T> {
    /// The field was observed.
    Known {
        /// Observed value.
        value: T,
    },
    /// The field was not observed; this says nothing about whether a process exited.
    Unsupported {
        /// Why it was not observed.
        reason: OpenEnum<UnsupportedReason>,
    },
}

impl<T> Observed<T> {
    /// An observed value.
    #[must_use]
    pub const fn known(value: T) -> Self {
        Self::Known { value }
    }

    /// A field not observed for `reason`.
    #[must_use]
    pub const fn unsupported(reason: UnsupportedReason) -> Self {
        Self::Unsupported {
            reason: OpenEnum::Known(reason),
        }
    }

    /// The observed value, if any.
    #[must_use]
    pub const fn value(&self) -> Option<&T> {
        match self {
            Self::Known { value } => Some(value),
            Self::Unsupported { .. } => None,
        }
    }
}

/// One native process, named by PID and the start token that rules out PID reuse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ProcessSample {
    /// Operating-system process identifier.
    pub pid: ProcessPid,
    /// Start token read together with the PID.
    pub start_token: StartToken,
    /// Current argument vector.
    pub argv: Observed<Vec<String>>,
    /// Current working directory.
    pub cwd: Observed<String>,
}

impl ProcessSample {
    /// A process with no argv or cwd observed yet.
    #[must_use]
    pub const fn new(pid: ProcessPid, start_token: StartToken, reason: UnsupportedReason) -> Self {
        Self {
            pid,
            start_token,
            argv: Observed::unsupported(reason),
            cwd: Observed::unsupported(reason),
        }
    }

    fn names(&self, pid: ProcessPid, start_token: &StartToken) -> bool {
        self.pid == pid && &self.start_token == start_token
    }
}

/// Shell, launcher, foreground and background processes of one terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ProcessRoles {
    /// The declared shell; never inferred from a process name.
    pub shell: Observed<ProcessSample>,
    /// The program the holder launched.
    pub launcher: Observed<ProcessSample>,
    /// Members of the terminal's foreground process group.
    pub foreground: Observed<Vec<ProcessSample>>,
    /// Other processes of the terminal's session.
    pub background: Observed<Vec<ProcessSample>>,
}

/// Wire names of the role fields, in `supported_fields` order.
const ROLE_FIELDS: [&str; 4] = ["shell", "launcher", "foreground", "background"];

impl ProcessRoles {
    /// Every role unsupported for one `reason`.
    #[must_use]
    pub const fn unsupported(reason: UnsupportedReason) -> Self {
        Self {
            shell: Observed::unsupported(reason),
            launcher: Observed::unsupported(reason),
            foreground: Observed::unsupported(reason),
            background: Observed::unsupported(reason),
        }
    }

    /// Names of the fields this observation reports: roles, then launcher facts.
    #[must_use]
    pub fn supported_fields(&self) -> Vec<String> {
        let roles = [
            self.shell.value().is_some(),
            self.launcher.value().is_some(),
            self.foreground.value().is_some(),
            self.background.value().is_some(),
        ];
        let mut fields: Vec<String> = ROLE_FIELDS
            .iter()
            .zip(roles)
            .filter(|(_, known)| *known)
            .map(|(name, _)| (*name).to_owned())
            .collect();
        if let Some(launcher) = self.launcher.value() {
            fields.extend(["pid".to_owned(), "start_token".to_owned()]);
            if launcher.argv.value().is_some() {
                fields.push("argv".to_owned());
            }
            if launcher.cwd.value().is_some() {
                fields.push("cwd".to_owned());
            }
        }
        fields
    }

    fn in_foreground(&self, pid: ProcessPid, start_token: &StartToken) -> bool {
        self.foreground
            .value()
            .is_some_and(|members| members.iter().any(|sample| sample.names(pid, start_token)))
    }
}

impl TerminalProcessInfoResult {
    /// The roles of `terminal`, whose launched program is `launcher`.
    ///
    /// `foreground` repeats the launcher record only while the launcher is observed in
    /// the foreground group; `roles.foreground` lists every member.
    #[must_use]
    pub fn new(terminal_id: TerminalId, launcher: ProcessRecord, roles: ProcessRoles) -> Self {
        let foreground = match (&launcher.pid, &launcher.start_token) {
            (Some(pid), Some(token)) if roles.in_foreground(*pid, token) => Some(launcher.clone()),
            _ => None,
        };
        Self {
            terminal_id,
            supported_fields: roles.supported_fields(),
            launcher,
            foreground,
            roles,
        }
    }
}

#[cfg(test)]
mod tests;
