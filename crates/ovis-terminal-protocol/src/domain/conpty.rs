//! ConPTY launch admission (plan todo 53).
//!
//! A Windows pseudoconsole takes its grid as a signed 16-bit `COORD` and starts its
//! program from one UTF-16 command line and environment block, both NUL-terminated. A
//! [`ConptyLaunch`] is checked against those limits before any pseudoconsole is
//! allocated, so a malformed `terminal.run` is refused without touching the OS. The
//! checks are platform-neutral and run on every host.

use std::collections::BTreeMap;
use std::path::PathBuf;

use thiserror::Error;

use crate::schema::methods::terminal::{LaunchProgram, TerminalRunParams};

/// Largest row or column count a pseudoconsole `COORD` can carry (`i16::MAX`).
pub const MAX_CONPTY_DIMENSION: u16 = 0x7fff;

/// A pseudoconsole grid: both dimensions from 1 to [`MAX_CONPTY_DIMENSION`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConptyGrid {
    cols: u16,
    rows: u16,
}

impl ConptyGrid {
    /// Checks a grid without clamping it.
    ///
    /// # Errors
    ///
    /// [`ConptyLaunchError::InvalidGeometry`] when either dimension is zero or above
    /// [`MAX_CONPTY_DIMENSION`].
    pub const fn new(cols: u16, rows: u16) -> Result<Self, ConptyLaunchError> {
        if cols == 0 || rows == 0 || cols > MAX_CONPTY_DIMENSION || rows > MAX_CONPTY_DIMENSION {
            return Err(ConptyLaunchError::InvalidGeometry { cols, rows });
        }
        Ok(Self { cols, rows })
    }

    /// Column count.
    #[must_use]
    pub const fn cols(self) -> u16 {
        self.cols
    }

    /// Row count.
    #[must_use]
    pub const fn rows(self) -> u16 {
        self.rows
    }
}

/// A launch a pseudoconsole can start: explicit argv, working directory, the complete
/// environment the caller composed, and a checked grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConptyLaunch {
    argv: Vec<String>,
    cwd: PathBuf,
    env: BTreeMap<String, String>,
    grid: ConptyGrid,
}

impl ConptyLaunch {
    /// Builds a launch with an empty environment.
    ///
    /// # Errors
    ///
    /// [`ConptyLaunchError::EmptyArgv`], [`ConptyLaunchError::NulInArgument`] or
    /// [`ConptyLaunchError::InvalidCwd`].
    pub fn new(
        argv: Vec<String>,
        cwd: PathBuf,
        grid: ConptyGrid,
    ) -> Result<Self, ConptyLaunchError> {
        if argv.is_empty() {
            return Err(ConptyLaunchError::EmptyArgv);
        }
        if let Some(index) = argv.iter().position(|argument| argument.contains('\0')) {
            return Err(ConptyLaunchError::NulInArgument { index });
        }
        let text = cwd.to_string_lossy();
        if text.is_empty() || text.contains('\0') {
            return Err(ConptyLaunchError::InvalidCwd);
        }
        Ok(Self {
            argv,
            cwd,
            env: BTreeMap::new(),
            grid,
        })
    }

    /// Replaces the environment; nothing is inherited from the holder.
    ///
    /// # Errors
    ///
    /// [`ConptyLaunchError::InvalidEnvironment`] for an empty name, a name containing
    /// `=` after its first character, or a NUL in a name or value.
    pub fn with_env(mut self, env: BTreeMap<String, String>) -> Result<Self, ConptyLaunchError> {
        let rejected = env.iter().find(|(name, value)| {
            name.is_empty()
                || name.get(1..).is_some_and(|rest| rest.contains('='))
                || name.contains('\0')
                || value.contains('\0')
        });
        if let Some((name, _)) = rejected {
            return Err(ConptyLaunchError::InvalidEnvironment {
                name: name.escape_debug().to_string(),
            });
        }
        self.env = env;
        Ok(self)
    }

    /// Admits `terminal.run` parameters for a pseudoconsole; the grid is checked first.
    ///
    /// # Errors
    ///
    /// [`ConptyLaunchError::InvalidGeometry`] before anything else, then
    /// [`ConptyLaunchError::TemplateUnresolved`] for a template program, and the
    /// argv, working-directory and environment checks of [`Self::new`] and
    /// [`Self::with_env`].
    pub fn admit(params: &TerminalRunParams) -> Result<Self, ConptyLaunchError> {
        let grid = ConptyGrid::new(params.geometry.cols, params.geometry.rows)?;
        let Some(LaunchProgram::Argv(argv)) = &params.program else {
            return Err(ConptyLaunchError::TemplateUnresolved);
        };
        let cwd = params.cwd.clone().ok_or(ConptyLaunchError::InvalidCwd)?;
        Self::new(argv.clone(), PathBuf::from(cwd), grid)?
            .with_env(params.env.clone().unwrap_or_default())
    }

    /// Program (first element) and arguments.
    #[must_use]
    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    /// Working directory the program starts in.
    #[must_use]
    pub const fn cwd(&self) -> &PathBuf {
        &self.cwd
    }

    /// The program's complete environment.
    #[must_use]
    pub const fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    /// Initial grid.
    #[must_use]
    pub const fn grid(&self) -> ConptyGrid {
        self.grid
    }
}

/// A launch a pseudoconsole cannot start; nothing was allocated.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConptyLaunchError {
    /// A grid dimension is zero or does not fit a pseudoconsole coordinate.
    #[error("grid {cols}x{rows} is outside 1..=32767 in a dimension")]
    InvalidGeometry {
        /// Requested columns.
        cols: u16,
        /// Requested rows.
        rows: u16,
    },
    /// The program is a template that has not been resolved to an argument vector.
    #[error("the program names a template, not an argument vector")]
    TemplateUnresolved,
    /// The argument vector names no program.
    #[error("the argument vector names no program")]
    EmptyArgv,
    /// An argument cannot be placed on a NUL-terminated command line.
    #[error("argument {index} contains a NUL character")]
    NulInArgument {
        /// Position of the argument in argv.
        index: usize,
    },
    /// The working directory is absent, empty or contains a NUL character.
    #[error("the working directory is absent, empty or contains a NUL character")]
    InvalidCwd,
    /// An environment entry cannot be placed in a Windows environment block.
    #[error("environment entry {name} cannot be placed in an environment block")]
    InvalidEnvironment {
        /// The entry's name, escaped.
        name: String,
    },
}

impl ConptyLaunchError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::InvalidGeometry { .. } => "invalid_geometry",
            Self::TemplateUnresolved
            | Self::EmptyArgv
            | Self::NulInArgument { .. }
            | Self::InvalidCwd
            | Self::InvalidEnvironment { .. } => "invalid_params",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::InvalidGeometry { .. } => "request between 1 and 32767 columns and rows",
            Self::TemplateUnresolved => "resolve the template to an argument vector first",
            Self::EmptyArgv => "pass the program as the first argv element",
            Self::NulInArgument { .. } => "remove NUL characters from the arguments",
            Self::InvalidCwd => "pass an existing directory as the working directory",
            Self::InvalidEnvironment { .. } => {
                "use non-empty names without '=' or NUL, and values without NUL"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::json;

    use super::{ConptyGrid, ConptyLaunch, ConptyLaunchError, MAX_CONPTY_DIMENSION};
    use crate::schema::methods::terminal::TerminalRunParams;

    fn params(overrides: &serde_json::Value) -> TerminalRunParams {
        let mut value = json!({
            "op_id": "0191f1a2-0000-7000-8000-000000000053",
            "program": {"argv": ["cmd.exe", "/d", "/c", "echo ovis"]},
            "geometry": {"cols": 80, "rows": 24},
            "cwd": "C:\\Users\\runner",
            "env": {"SystemRoot": "C:\\Windows"},
        });
        let fields = value.as_object_mut().expect("object");
        for (key, field) in overrides.as_object().into_iter().flatten() {
            fields.insert(key.clone(), field.clone());
        }
        serde_json::from_value(value).expect("terminal.run params")
    }

    proptest! {
        #[test]
        fn a_grid_is_admitted_exactly_when_both_dimensions_fit_a_coord(cols in any::<u16>(), rows in any::<u16>()) {
            let fits = |value: u16| (1..=MAX_CONPTY_DIMENSION).contains(&value);
            match ConptyGrid::new(cols, rows) {
                Ok(grid) => {
                    prop_assert!(fits(cols) && fits(rows));
                    prop_assert_eq!((grid.cols(), grid.rows()), (cols, rows));
                    prop_assert!(i16::try_from(grid.cols()).is_ok() && i16::try_from(grid.rows()).is_ok());
                }
                Err(error) => {
                    prop_assert!(!(fits(cols) && fits(rows)));
                    prop_assert_eq!(error, ConptyLaunchError::InvalidGeometry { cols, rows });
                }
            }
        }
    }

    #[test]
    fn geometry_is_refused_before_any_other_defect() {
        let broken = params(&json!({"geometry": {"cols": 80, "rows": 0}, "program": {"argv": []}}));
        let error = ConptyLaunch::admit(&broken).expect_err("rows 0 is refused");
        assert_eq!(
            error,
            ConptyLaunchError::InvalidGeometry { cols: 80, rows: 0 }
        );
        assert_eq!(error.kind(), "invalid_geometry");
        let wide = params(&json!({"geometry": {"cols": 32768, "rows": 24}}));
        assert_eq!(
            ConptyLaunch::admit(&wide).map_err(|error| error.kind()),
            Err("invalid_geometry"),
            "a column count that would wrap a signed COORD is refused, not clamped"
        );
    }

    #[test]
    fn malformed_programs_environments_and_directories_are_refused_with_their_cause() {
        let env = |name: &str| ConptyLaunchError::InvalidEnvironment {
            name: name.to_owned(),
        };
        let template = json!({"template": {"name": "shell", "version": null, "task": null}});
        let cases = [
            (
                json!({"program": template}),
                ConptyLaunchError::TemplateUnresolved,
            ),
            (
                json!({"program": {"argv": []}}),
                ConptyLaunchError::EmptyArgv,
            ),
            (
                json!({"program": {"argv": ["cmd.exe", "a\u{0}b"]}}),
                ConptyLaunchError::NulInArgument { index: 1 },
            ),
            (json!({"cwd": null}), ConptyLaunchError::InvalidCwd),
            (json!({"cwd": ""}), ConptyLaunchError::InvalidCwd),
            (json!({"env": {"A=B": "x"}}), env("A=B")),
            (json!({"env": {"": "x"}}), env("")),
            (json!({"env": {"PATH": "a\u{0}"}}), env("PATH")),
        ];
        for (overrides, expected) in cases {
            let error = ConptyLaunch::admit(&params(&overrides)).expect_err("refused");
            assert_eq!(error, expected, "{overrides}");
            assert_eq!(error.kind(), "invalid_params");
            assert!(!error.to_string().contains(error.kind()));
        }
    }

    #[test]
    fn an_admitted_launch_keeps_every_field_and_drive_relative_names_are_allowed() {
        let env = json!({"SystemRoot": "C:\\Windows", "=C:": "C:\\work"});
        let launch = ConptyLaunch::admit(&params(&json!({"env": env}))).expect("admitted");
        assert_eq!(launch.argv(), ["cmd.exe", "/d", "/c", "echo ovis"]);
        assert_eq!(launch.cwd().to_string_lossy(), "C:\\Users\\runner");
        assert_eq!(
            launch.env().get("=C:").map(String::as_str),
            Some("C:\\work")
        );
        assert_eq!((launch.grid().cols(), launch.grid().rows()), (80, 24));
    }
}
