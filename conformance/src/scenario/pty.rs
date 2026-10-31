//! Steps that run the binary on a real outer PTY, the terminal an attach client owns.

use serde::Deserialize;

use super::matcher::Expect;
use crate::evidence::Descriptors;

/// Start the binary with a fresh PTY as its controlling terminal, stdin and stdout.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PtyOpen {
    pub(crate) pty: String,
    pub(crate) args: Vec<String>,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
}

/// Type these bytes on the outer terminal.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PtyType {
    pub(crate) pty: String,
    pub(crate) bytes: String,
}

/// Require `output` on the outer terminal; with `retry_type`, type it again until it shows.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PtyExpect {
    pub(crate) pty: String,
    pub(crate) output: String,
    #[serde(default)]
    pub(crate) retry_type: Option<String>,
}

/// Change the outer terminal's grid; the kernel signals the client's foreground group.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PtyResize {
    pub(crate) pty: String,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
}

/// Await the client's exit; its code, stderr record and restored terminal settings are checked.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PtyExit {
    pub(crate) pty: String,
    pub(crate) exit: i32,
    #[serde(default)]
    pub(crate) stderr: Expect,
}

fn planned<const N: usize>(fields: [(&str, String); N]) -> Descriptors {
    fields
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

impl PtyOpen {
    /// What this step does, recorded in the evidence on every platform.
    pub(crate) fn planned(&self) -> Descriptors {
        planned([
            ("pty", self.pty.clone()),
            ("args", self.args.join(" ")),
            ("grid", format!("{}x{}", self.cols, self.rows)),
        ])
    }
}

impl PtyType {
    /// What this step does, recorded in the evidence on every platform.
    pub(crate) fn planned(&self) -> Descriptors {
        planned([
            ("pty", self.pty.clone()),
            ("bytes", self.bytes.escape_default().to_string()),
        ])
    }
}

impl PtyExpect {
    /// What this step does, recorded in the evidence on every platform.
    pub(crate) fn planned(&self) -> Descriptors {
        let retry = self
            .retry_type
            .as_deref()
            .unwrap_or_default()
            .escape_default()
            .to_string();
        planned([
            ("pty", self.pty.clone()),
            ("output", self.output.clone()),
            ("retry_type", retry),
        ])
    }
}

impl PtyResize {
    /// What this step does, recorded in the evidence on every platform.
    pub(crate) fn planned(&self) -> Descriptors {
        planned([
            ("pty", self.pty.clone()),
            ("grid", format!("{}x{}", self.cols, self.rows)),
        ])
    }
}

impl PtyExit {
    /// What this step does, recorded in the evidence on every platform.
    pub(crate) fn planned(&self) -> Descriptors {
        planned([
            ("pty", self.pty.clone()),
            ("exit", self.exit.to_string()),
            ("stderr", format!("{:?}", self.stderr)),
        ])
    }
}
