//! Literal launch argument vectors and environments; no value is ever shell-interpolated.

use std::collections::BTreeMap;

use super::violation::{BoundaryViolation, Malformation, PayloadLimit};

/// Prefix of the variables the runtime sets itself, such as the process report token.
pub const RESERVED_ENV_PREFIX: &str = "OVIS_TERMINAL_";

const ARGV: &str = "params.program.argv";
const ENV: &str = "params.env";

/// A program and its arguments exactly as the OS receives them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchArgv(Vec<String>);

impl LaunchArgv {
    /// Accepts a non-empty vector within the argv limits and without NUL bytes.
    ///
    /// Shell metacharacters are ordinary bytes here: the vector is handed to the OS as is.
    ///
    /// # Errors
    ///
    /// [`BoundaryViolation::Malformed`] for an empty vector, an empty program or a NUL
    /// byte; [`BoundaryViolation::TooLarge`] past an argv limit.
    pub fn parse(argv: &[String]) -> Result<Self, BoundaryViolation> {
        let malformed = |reason| BoundaryViolation::Malformed {
            field: ARGV,
            reason,
        };
        if argv.first().is_none_or(String::is_empty) {
            return Err(malformed(Malformation::EmptyArgv));
        }
        PayloadLimit::ArgvElements.check(ARGV, argv.len())?;
        let mut total = 0_usize;
        for argument in argv {
            if argument.contains('\0') {
                return Err(malformed(Malformation::NulByte));
            }
            PayloadLimit::ArgvElementBytes.check(ARGV, argument.len())?;
            total = total.saturating_add(argument.len());
        }
        PayloadLimit::ArgvTotalBytes.check(ARGV, total)?;
        Ok(Self(argv.to_vec()))
    }

    /// The literal vector.
    #[must_use]
    pub fn as_slice(&self) -> &[String] {
        &self.0
    }
}

/// Caller-supplied environment additions, none of them reserved by the runtime.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LaunchEnvironment(BTreeMap<String, String>);

impl LaunchEnvironment {
    /// Accepts variables within the environment limits, refusing reserved names.
    ///
    /// Names compare against [`RESERVED_ENV_PREFIX`] after Unicode uppercasing, because Windows
    /// environment names compare case-insensitively: `ovıs_terminal_x`, spelt with a dotless
    /// i, names the same variable there and is reserved too.
    ///
    /// # Errors
    ///
    /// [`BoundaryViolation::ReservedEnvironment`] for a reserved name,
    /// [`BoundaryViolation::Malformed`] for an empty name, `=` in a name or a NUL byte,
    /// and [`BoundaryViolation::TooLarge`] past an environment limit.
    pub fn parse(env: &BTreeMap<String, String>) -> Result<Self, BoundaryViolation> {
        PayloadLimit::EnvEntries.check(ENV, env.len())?;
        for (key, value) in env {
            if key.is_empty() || key.contains('=') {
                return Err(BoundaryViolation::Malformed {
                    field: ENV,
                    reason: Malformation::EnvKey,
                });
            }
            if key.contains('\0') || value.contains('\0') {
                return Err(BoundaryViolation::Malformed {
                    field: ENV,
                    reason: Malformation::NulByte,
                });
            }
            PayloadLimit::EnvKeyBytes.check(ENV, key.len())?;
            PayloadLimit::EnvValueBytes.check(ENV, value.len())?;
            if key.to_uppercase().starts_with(RESERVED_ENV_PREFIX) {
                return Err(BoundaryViolation::ReservedEnvironment { field: ENV });
            }
        }
        Ok(Self(env.clone()))
    }

    /// The accepted variables.
    #[must_use]
    pub const fn as_map(&self) -> &BTreeMap<String, String> {
        &self.0
    }
}
