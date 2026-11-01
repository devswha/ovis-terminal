//! The runtime never holds the control while it waits: with its idle wait stretched to ten
//! minutes, a spawn and a lifecycle read are still answered at once. Were the wait taken
//! inside the control (the starvation fixed in round 4), the actor would sit on the
//! control for the whole wait and both would miss their deadline. The stretched wait
//! makes that failure certain rather than a matter of scheduling.

use super::support::{Served, TestResult};

#[test]
fn a_spawn_and_a_lifecycle_read_are_served_while_the_runtime_waits_for_requests() -> TestResult {
    let served = Served::start_with(&["--holder-poll-ms", "600000"])?;
    served.run_shell("shell")?;
    let query = served.ok(&["call", "update.query", "--params", "{}"])?;
    assert!(query.get("active").is_some(), "{query}");
    served.stop()?;
    Ok(())
}
