//! The one check of a launch's caller-supplied argv and environment.

use ovis_terminal_protocol::domain::boundary_policy::{
    BoundaryViolation, LaunchArgv, LaunchEnvironment,
};
use ovis_terminal_protocol::schema::methods::terminal::{LaunchProgram, TerminalRunParams};

use crate::dispatch::DispatchError;

/// The dispatch failure of a boundary refusal, keeping its own kind and hint.
pub fn violation(violation: BoundaryViolation) -> DispatchError {
    DispatchError::Domain {
        kind: violation.kind(),
        hint: violation.hint(),
        source: Box::new(violation),
    }
}

/// Accepts a launch with a literal, bounded argv and no runtime-reserved variable.
///
/// It runs before the launch is admitted, so a refusal leaves no operation, terminal or
/// file behind.
///
/// # Errors
///
/// The [`BoundaryViolation`] kind: `invalid_params`, `too_large` or `forbidden`.
pub fn admit_launch(params: &TerminalRunParams) -> Result<(), DispatchError> {
    if let Some(LaunchProgram::Argv(argv)) = &params.program {
        LaunchArgv::parse(argv).map_err(violation)?;
    }
    if let Some(env) = &params.env {
        LaunchEnvironment::parse(env).map_err(violation)?;
    }
    Ok(())
}
