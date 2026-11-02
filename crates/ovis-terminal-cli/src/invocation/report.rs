//! What an invocation writes: a result on stdout or one error on stderr, and its status.

use std::io::{self, Write as _};
use std::process::ExitCode;

use ovis_terminal_protocol::rpc::RpcErrorObject;
use serde_json::{Map, Value, json};

use super::context::OutputMode;
use super::error::CliError;
use super::exit::ExitStatus;
use super::text::render;

mod record;

use record::cli_record;

/// A failure of this binary, or a typed error a server returned.
#[derive(Debug)]
pub(super) enum Failure {
    Cli(CliError),
    Server(Box<RpcErrorObject>),
}

impl From<CliError> for Failure {
    fn from(error: CliError) -> Self {
        Self::Cli(error)
    }
}

impl Failure {
    fn status(&self) -> ExitStatus {
        match self {
            Self::Cli(error) => ExitStatus::for_kind(error.kind()),
            Self::Server(error) => ExitStatus::for_kind(error.data.kind.token()),
        }
    }

    fn record(&self) -> Result<Map<String, Value>, serde_json::Error> {
        let mut record = match self {
            Self::Cli(error) => cli_record(error),
            Self::Server(error) => {
                let Value::Object(mut data) = serde_json::to_value(&error.data)? else {
                    return Err(serde::ser::Error::custom("error data is not an object"));
                };
                data.insert("origin".to_owned(), json!("server"));
                data.insert("message".to_owned(), json!(error.message));
                data.insert("code".to_owned(), json!(error.code));
                data
            }
        };
        record.insert("exit_code".to_owned(), json!(self.status().code()));
        Ok(record)
    }

    fn line(&self) -> String {
        let (message, kind, hint) = match self {
            Self::Cli(error) => (error.to_string(), error.kind(), error.hint().to_owned()),
            Self::Server(error) => (
                error.message.clone(),
                error.data.kind.token(),
                error.data.hint.clone(),
            ),
        };
        let mut text = format!("ovis-terminal: {message} [{kind}]\nhint: {hint}\n");
        if let Self::Cli(CliError::NeedsTarget { candidates, .. }) = self {
            let listed: String = candidates
                .iter()
                .map(|candidate| {
                    [
                        "candidate: ",
                        candidate.terminal_id.as_str(),
                        " machine=",
                        candidate.machine_id.as_str(),
                        " lifetime=",
                        candidate.runtime_lifetime.as_str(),
                        "\n",
                    ]
                    .concat()
                })
                .collect();
            text.push_str(&listed);
        }
        if let Self::Cli(CliError::CompatibilityBreak { breaks }) = self {
            for change in breaks {
                text.push_str(
                    &[
                        "break: ",
                        change.rule.token(),
                        " at ",
                        &change.path,
                        "
",
                    ]
                    .concat(),
                );
            }
        }
        text
    }
}

/// The bytes an invocation writes and the status it exits with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    status: ExitStatus,
}

impl Report {
    /// A successful structured result in the requested form.
    pub(super) fn success(mode: OutputMode, value: &Value) -> Self {
        let text = match mode {
            OutputMode::Json => format!("{value}\n"),
            OutputMode::Text => render(value),
        };
        Self::raw(text)
    }

    /// A successful result printed exactly as given.
    pub(super) const fn raw(text: String) -> Self {
        Self {
            stdout: text.into_bytes(),
            stderr: Vec::new(),
            status: ExitStatus::Success,
        }
    }

    /// A success whose stdout was the attached screen: the outcome record goes to stderr.
    #[cfg(unix)]
    pub(super) fn detached(record: &Value) -> Self {
        Self {
            stdout: Vec::new(),
            stderr: format!("{record}\n").into_bytes(),
            status: ExitStatus::Success,
        }
    }

    /// A failure in the requested form; stdout stays empty.
    pub(super) fn failure(mode: OutputMode, failure: &Failure) -> Self {
        let status = failure.status();
        let text = match mode {
            OutputMode::Text => Ok(failure.line()),
            OutputMode::Json => failure
                .record()
                .map(|record| format!("{}\n", json!({ "error": record }))),
        };
        match text {
            Ok(text) => Self {
                stdout: Vec::new(),
                stderr: text.into_bytes(),
                status,
            },
            Err(source) => Self::failure(OutputMode::Text, &CliError::Encode(source).into()),
        }
    }

    /// Bytes for stdout.
    #[must_use]
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    /// Bytes for stderr.
    #[must_use]
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    /// The exit status.
    #[must_use]
    pub const fn status(&self) -> ExitStatus {
        self.status
    }

    /// Writes both streams and returns the exit code; a write failure exits 1.
    #[must_use]
    pub fn emit(&self) -> ExitCode {
        let written = io::stdout()
            .lock()
            .write_all(&self.stdout)
            .and_then(|()| io::stdout().lock().flush())
            .and_then(|()| io::stderr().lock().write_all(&self.stderr));
        match written {
            Ok(()) => self.status.into(),
            Err(_) => ExitStatus::Failure.into(),
        }
    }
}
