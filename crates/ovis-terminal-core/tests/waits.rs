//! Atomic cursor waits and evidenced interrupt outcomes (plan todo 27), against an
//! in-memory screen and, on Unix, a real holder PTY driven through the input sequencer.

use std::error::Error;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

#[path = "../src/waits/qa/fixture.rs"]
pub mod fixture;
#[path = "../src/waits/qa/interrupt.rs"]
pub mod interrupt;
#[cfg(unix)]
#[path = "../src/waits/qa/live.rs"]
pub mod live;
#[path = "../src/waits/qa/output.rs"]
pub mod output;
#[cfg(unix)]
#[path = "../src/waits/qa/pty.rs"]
pub mod pty;
#[cfg(unix)]
#[path = "../src/input/sequencer/qa/fixture.rs"]
pub mod sequencer;
#[path = "../src/waits/qa/state.rs"]
pub mod state;
#[path = "../src/waits/qa/turn.rs"]
pub mod turn;

/// Happy path on a real PTY: `READY_MARK` echoed during the send is matched from the receipt cursor.
#[cfg(unix)]
#[test]
fn happy() -> TestResult {
    live::ready_mark_printed_during_send_is_matched_from_the_receipt_cursor()
}

/// Failure path on a real PTY: Escape to a running program is delivered but never `stopped`.
#[cfg(unix)]
#[test]
fn failure() -> TestResult {
    live::escape_to_a_running_program_is_delivered_but_not_stopped()
}

#[cfg(unix)]
#[test]
fn interrupt_that_ends_the_program_is_stopped_by_exit_evidence() -> TestResult {
    live::interrupt_that_ends_the_program_is_stopped_by_exit_evidence()
}

#[test]
fn marker_printed_before_arming_is_matched_from_the_cursor() -> TestResult {
    output::marker_printed_before_arming_is_matched_from_the_cursor()
}

#[test]
fn armed_wait_wakes_on_a_marker_split_across_chunks() -> TestResult {
    output::armed_wait_wakes_on_a_marker_split_across_chunks()
}

#[test]
fn expired_cursor_is_rejected_as_output_gap() -> TestResult {
    output::expired_cursor_is_rejected_as_output_gap()
}

#[test]
fn exit_and_cancel_end_output_waits_distinctly() -> TestResult {
    output::exit_and_cancel_end_output_waits_distinctly()
}

#[test]
fn outcomes_round_trip_decimal_offsets() -> TestResult {
    output::outcomes_round_trip_decimal_offsets()
}

#[test]
fn already_satisfied_state_returns_immediately() -> TestResult {
    state::already_satisfied_state_returns_immediately()
}

#[test]
fn armed_state_wait_ends_by_report_exit_loss_or_timeout() -> TestResult {
    state::armed_state_wait_ends_by_report_exit_loss_or_timeout()
}

#[test]
fn stale_and_late_reports_are_rejected() -> TestResult {
    state::stale_and_late_reports_are_rejected()
}

#[test]
fn cancel_scope_ends_and_detaches_its_waits() -> TestResult {
    state::cancel_scope_ends_and_detaches_its_waits()
}

#[test]
fn only_the_turn_after_the_send_completes_its_wait() -> TestResult {
    turn::only_the_turn_after_the_send_completes_its_wait()
}

#[test]
fn timeout_exit_and_cancel_never_complete_a_turn() -> TestResult {
    turn::timeout_exit_and_cancel_never_complete_a_turn()
}

#[test]
fn unknown_and_stale_turns_are_rejected() -> TestResult {
    turn::unknown_and_stale_turns_are_rejected()
}

#[test]
fn declared_stop_after_arming_is_stop_evidence() -> TestResult {
    interrupt::declared_stop_after_arming_is_stop_evidence()
}

#[test]
fn text_and_earlier_status_never_claim_a_stop() -> TestResult {
    interrupt::text_and_earlier_status_never_claim_a_stop()
}

#[test]
fn exit_stops_and_cancel_does_not() -> TestResult {
    interrupt::exit_stops_and_cancel_does_not()
}
