//! ConPTY admission and spawn for `terminal.run` on Windows (plan todo 53).
//!
//! [`admit`] refuses a launch a pseudoconsole cannot start before anything is
//! allocated, and on Windows `spawn` starts an admitted launch, reporting a program
//! that could not start as `spawn_failed`. Both carry the domain's own kind and hint to
//! the wire. The served `terminal.run` route that calls them is assembled by plan
//! todos 31 and 54.

use ovis_terminal_protocol::domain::conpty::{ConptyLaunch, ConptyLaunchError};
use ovis_terminal_protocol::schema::methods::terminal::TerminalRunParams;

use crate::dispatch::DispatchError;

/// Checks `terminal.run` parameters against pseudoconsole limits; the grid comes first.
///
/// # Errors
///
/// [`DispatchError::Domain`] with `invalid_geometry` or `invalid_params`.
pub fn admit(params: &TerminalRunParams) -> Result<ConptyLaunch, DispatchError> {
    ConptyLaunch::admit(params).map_err(|error: ConptyLaunchError| DispatchError::Domain {
        kind: error.kind(),
        hint: error.hint(),
        source: Box::new(error),
    })
}

/// Starts an admitted launch on a new pseudoconsole owned by this process.
///
/// # Errors
///
/// [`DispatchError::Domain`] with the terminal error's kind, `spawn_failed` when no
/// program was started.
#[cfg(windows)]
pub fn spawn(
    launch: &ConptyLaunch,
    limits: ovis_terminal_holder::windows::TerminalLimits,
) -> Result<ovis_terminal_holder::windows::ConptyTerminal, DispatchError> {
    ovis_terminal_holder::windows::ConptyTerminal::spawn(launch, limits).map_err(|error| {
        DispatchError::Domain {
            kind: error.kind(),
            hint: error.hint(),
            source: Box::new(error),
        }
    })
}

#[cfg(test)]
mod tests {
    use ovis_terminal_protocol::schema::methods::terminal::TerminalRunParams;
    use serde_json::{Value, json};

    use super::admit;

    fn params(geometry: &Value, argv: &Value) -> TerminalRunParams {
        serde_json::from_value(json!({
            "op_id": "0191f1a2-0000-7000-8000-000000000053",
            "program": {"argv": argv},
            "geometry": geometry,
            "cwd": "C:\\Users\\runner",
        }))
        .expect("terminal.run params")
    }

    #[test]
    fn a_zero_row_launch_is_invalid_geometry_on_the_wire_before_anything_else() {
        let error = admit(&params(&json!({"cols": 80, "rows": 0}), &json!([])))
            .expect_err("rows 0 is refused");
        assert_eq!(error.kind(), "invalid_geometry");
        let wire = error.boundary();
        assert_eq!(wire.kind(), "invalid_geometry");
        assert!(std::error::Error::source(&error).is_some());
    }

    #[test]
    fn malformed_argv_is_invalid_params_and_a_valid_launch_is_admitted() {
        let empty = admit(&params(&json!({"cols": 80, "rows": 24}), &json!([])));
        assert_eq!(
            empty.map_err(|error| error.kind()).err(),
            Some("invalid_params")
        );
        let launch = admit(&params(
            &json!({"cols": 32767, "rows": 1}),
            &json!(["cmd.exe"]),
        ))
        .expect("admitted");
        assert_eq!((launch.grid().cols(), launch.grid().rows()), (32767, 1));
    }

    #[cfg(windows)]
    #[test]
    fn a_program_that_cannot_start_is_spawn_failed() {
        use ovis_terminal_holder::windows::TerminalLimits;
        use ovis_terminal_protocol::domain::conpty::{ConptyGrid, ConptyLaunch};

        let grid = ConptyGrid::new(80, 24).expect("grid");
        let argv = vec![r"C:\ovis-missing\absent.exe".to_owned()];
        let launch = ConptyLaunch::new(argv, std::env::temp_dir(), grid).expect("launch");
        let error = super::spawn(&launch, TerminalLimits::DEFAULT).expect_err("no such program");
        assert_eq!(error.kind(), "spawn_failed");
    }
}
