//! Agent-facing ovis-terminal command-line interface.
//!
//! The scaffold binary answers `--version` only; every other argument list is a
//! usage error. The JSON command surface arrives with plan todo 20.

use std::ffi::OsString;

/// Name of the installed binary.
pub const BINARY_NAME: &str = "ovis-terminal";

/// Release version of the binary, taken from the workspace package version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Cargo package name of this crate, used by the workspace layering record.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Workspace crates this crate builds on; every entry is a declared Cargo dependency.
pub const DEPENDS_ON: &[&str] = &[
    ovis_terminal_protocol::CRATE_NAME,
    ovis_terminal_server::CRATE_NAME,
];

/// Exit status for a command line that could not be parsed.
pub const USAGE_EXIT_CODE: u8 = 2;

/// A parsed, supported invocation of the binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "the binary matches every invocation; a new one must break that match"
)]
pub enum Invocation {
    /// Print the version line and exit successfully.
    Version,
}

/// Command-line arguments the binary does not accept.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum UsageError {
    /// The argument list is not exactly `--version`.
    #[error("unsupported arguments: this build accepts only `--version`")]
    Unsupported,
}

impl Invocation {
    /// Parses the arguments that follow the program name.
    ///
    /// # Errors
    ///
    /// Returns [`UsageError::Unsupported`] for any argument list other than
    /// exactly `--version`.
    pub fn parse(args: &[OsString]) -> Result<Self, UsageError> {
        match args {
            [only] if only == "--version" => Ok(Self::Version),
            _ => Err(UsageError::Unsupported),
        }
    }
}

/// The line printed by `ovis-terminal --version`, for example `ovis-terminal 0.1.0`.
pub fn version_line() -> String {
    format!("{BINARY_NAME} {VERSION}")
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{Invocation, UsageError, version_line};

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn version_flag_alone_parses() {
        assert_eq!(
            Invocation::parse(&args(&["--version"])),
            Ok(Invocation::Version)
        );
    }

    #[test]
    fn empty_and_extra_arguments_are_usage_errors() {
        assert_eq!(Invocation::parse(&args(&[])), Err(UsageError::Unsupported));
        assert_eq!(
            Invocation::parse(&args(&["--version", "--json"])),
            Err(UsageError::Unsupported)
        );
        assert_eq!(
            Invocation::parse(&args(&["-V"])),
            Err(UsageError::Unsupported)
        );
    }

    #[test]
    fn version_line_names_binary_and_package_version() {
        assert_eq!(
            version_line(),
            format!("ovis-terminal {}", env!("CARGO_PKG_VERSION"))
        );
    }
}
