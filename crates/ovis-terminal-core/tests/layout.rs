//! Workspace, tab, pane and split-node layout against real SQLite and authority.

#[path = "../src/layout/qa/mod.rs"]
pub mod qa;

#[test]
fn happy() -> qa::TestResult {
    qa::happy()
}

#[test]
fn failure() -> qa::TestResult {
    qa::failure()
}

#[test]
fn malformed_requests_are_rejected_before_any_effect() -> qa::TestResult {
    qa::malformed_requests_are_rejected_before_any_effect()
}

#[test]
fn replay_converges_and_reuse_is_refused() -> qa::TestResult {
    qa::replay_converges_and_reuse_is_refused()
}

#[test]
fn session_snapshot_holds_layout_and_terminals_at_one_revision() -> qa::TestResult {
    qa::session_snapshot_holds_layout_and_terminals_at_one_revision()
}

#[cfg(unix)]
#[test]
fn launch_binds_terminal_to_created_pane() -> qa::TestResult {
    qa::launch_binds_terminal_to_created_pane()
}

#[cfg(unix)]
#[test]
fn layout_never_mutates_a_checkout() -> qa::TestResult {
    qa::layout_never_mutates_a_checkout()
}
