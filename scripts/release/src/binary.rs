//! Running the built `ovis-terminal` binary: version, manual, schema and released-baseline
//! compatibility, each kept as an invocation record for the evidence.

use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use crate::error::ReleaseError;
use crate::files::sha256_bytes;

/// One finished run of the binary.
#[derive(Debug, Clone)]
pub(crate) struct Invocation {
    /// The binary path followed by its arguments.
    pub(crate) argv: Vec<String>,
    /// The exit code, `-1` when a signal ended it.
    pub(crate) exit: i32,
    /// Everything it wrote to stdout.
    pub(crate) stdout: Vec<u8>,
    /// Everything it wrote to stderr.
    pub(crate) stderr: Vec<u8>,
}

fn captured(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or_else(
        |_| json!({ "sha256": sha256_bytes(bytes), "bytes": bytes.len().to_string() }),
    )
}

impl Invocation {
    /// argv, exit status and both streams: parsed JSON, or digest and length.
    #[must_use]
    pub(crate) fn record(&self) -> Value {
        json!({
            "argv": self.argv,
            "exit": self.exit,
            "stdout": captured(&self.stdout),
            "stderr": captured(&self.stderr),
        })
    }

    fn command(&self) -> String {
        self.argv.join(" ")
    }

    fn failed(&self) -> ReleaseError {
        ReleaseError::BinaryFailed {
            command: self.command(),
            exit: self.exit,
            binary: self.record(),
        }
    }

    fn json_success(&self) -> Result<Value, ReleaseError> {
        if self.exit != 0 {
            return Err(self.failed());
        }
        serde_json::from_slice(&self.stdout).map_err(|_| self.failed())
    }
}

/// Runs `binary` with `args`, with `OVIS_JSON` unset so only the flags choose the format.
///
/// # Errors
///
/// [`ReleaseError::Tool`] when the binary cannot start.
pub(crate) fn invoke(binary: &Path, args: &[&str]) -> Result<Invocation, ReleaseError> {
    let output = Command::new(binary)
        .args(args)
        .env_remove("OVIS_JSON")
        .output()
        .map_err(|error| ReleaseError::Tool {
            program: binary.display().to_string(),
            detail: error.to_string(),
        })?;
    let command_line = std::iter::once(binary.display().to_string())
        .chain(args.iter().map(|arg| (*arg).to_owned()))
        .collect();
    Ok(Invocation {
        argv: command_line,
        exit: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

/// What the binary reports about itself.
#[derive(Debug, Clone)]
pub(crate) struct Identity {
    /// `--json --version` version.
    pub(crate) version: String,
    /// `--json --skill` version.
    pub(crate) manual_version: String,
    /// The protocol generation `--version` reports.
    pub(crate) generation: String,
    /// The embedded manual.
    pub(crate) manual: String,
    /// `schema` output, byte for byte.
    pub(crate) schema: Vec<u8>,
}

fn text(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Reads version, manual and schema from `binary`.
///
/// # Errors
///
/// [`ReleaseError::BinaryFailed`] when a discovery command fails or prints no JSON.
pub(crate) fn identity(binary: &Path) -> Result<Identity, ReleaseError> {
    let version = invoke(binary, &["--json", "--version"])?.json_success()?;
    let skill = invoke(binary, &["--json", "--skill"])?.json_success()?;
    let schema = invoke(binary, &["schema"])?;
    if schema.exit != 0 {
        return Err(schema.failed());
    }
    Ok(Identity {
        version: text(&version, "/version"),
        manual_version: text(&skill, "/version"),
        generation: text(&version, "/generation"),
        manual: text(&skill, "/manual"),
        schema: schema.stdout,
    })
}

/// Runs `schema compat --released <released>` and returns its record when compatible.
///
/// # Errors
///
/// [`ReleaseError::CompatibilityBreak`] when the binary answers `compatibility_break`,
/// [`ReleaseError::BinaryFailed`] for any other failure.
pub(crate) fn compat(binary: &Path, released: &Path) -> Result<Value, ReleaseError> {
    let released = released.display().to_string();
    let run = invoke(
        binary,
        &["--json", "schema", "compat", "--released", &released],
    )?;
    let answered = serde_json::from_slice::<Value>(&run.stderr).ok();
    let kind = answered
        .as_ref()
        .and_then(|error| error.pointer("/error/kind"));
    if run.exit == 1 && kind == Some(&json!("compatibility_break")) {
        return Err(ReleaseError::CompatibilityBreak {
            binary: run.record(),
        });
    }
    let verdict = run.json_success()?;
    if verdict.get("compatible") == Some(&json!(true)) {
        Ok(run.record())
    } else {
        Err(run.failed())
    }
}
