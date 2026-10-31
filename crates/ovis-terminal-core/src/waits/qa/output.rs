//! Output waits against an in-memory screen: cursor boundary, retention gap, typed ends.

use std::thread;
use std::time::Instant;

use ovis_terminal_core::waits::{CancelScope, OutputOutcome, OutputWait};
use proptest::prelude::*;

use super::TestResult;
use super::fixture::{at, bound, exit, hub, print};

/// A marker printed after the cursor was captured but before the wait armed is matched.
///
/// # Errors
///
/// Fails when the hub refuses a step.
///
/// # Panics
///
/// When an asserted outcome does not hold.
pub fn marker_printed_before_arming_is_matched_from_the_cursor() -> TestResult {
    let (waits, _screen) = hub(4096)?;
    print(&waits, b"$ ")?;
    let cursor = waits.cursor()?;
    print(&waits, b"sleep 1; echo READY_MARK\r\nREADY_MARK\r\n")?;
    let ticket = waits.output(
        OutputWait::new(cursor, b"READY_MARK\r\n\r")?,
        &CancelScope::new(),
    )?;
    let late = waits.output(
        OutputWait::new(cursor, b"READY_MARK\r\n")?,
        &CancelScope::new(),
    )?;
    assert_eq!(
        late.wait(Instant::now())?,
        OutputOutcome::Matched {
            at: at(16),
            cursor: at(28)
        }
    );
    assert!(matches!(
        ticket.wait(Instant::now())?,
        OutputOutcome::Timeout { .. }
    ));
    let fresh = waits.output(
        OutputWait::new(waits.cursor()?, b"READY_MARK")?,
        &CancelScope::new(),
    )?;
    assert_eq!(
        fresh.wait(Instant::now())?,
        OutputOutcome::Timeout { cursor: at(40) },
        "output before the cursor never satisfies a wait"
    );
    Ok(())
}

/// An armed wait blocked on another thread wakes on the chunk that completes the marker.
///
/// # Errors
///
/// Fails when the hub refuses a step or the waiting thread panics.
///
/// # Panics
///
/// When an asserted outcome does not hold.
pub fn armed_wait_wakes_on_a_marker_split_across_chunks() -> TestResult {
    let (waits, _screen) = hub(4096)?;
    let ticket = waits.output(
        OutputWait::new(waits.cursor()?, b"READY")?,
        &CancelScope::new(),
    )?;
    let deadline = bound()?;
    let waiter = thread::spawn(move || ticket.wait(deadline));
    print(&waits, b"..RE")?;
    print(&waits, b"ADY..")?;
    let outcome = waiter.join().map_err(|_| "waiter panicked")??;
    assert_eq!(
        outcome,
        OutputOutcome::Matched {
            at: at(2),
            cursor: at(7)
        }
    );
    Ok(())
}

/// A cursor behind retention is rejected as `output_gap`, never answered with a timeout.
///
/// # Errors
///
/// Fails when the hub refuses a setup step.
///
/// # Panics
///
/// When an asserted outcome does not hold.
pub fn expired_cursor_is_rejected_as_output_gap() -> TestResult {
    let (waits, _screen) = hub(16)?;
    print(&waits, b"READY_MARK and then a lot more output")?;
    let Err(error) = waits.output(OutputWait::new(at(0), b"READY_MARK")?, &CancelScope::new())
    else {
        return Err("an evicted cursor was armed".into());
    };
    assert_eq!(error.kind(), "output_gap");
    assert!(
        error
            .to_string()
            .contains("retained output starts at byte 21")
    );
    let Err(ahead) = waits.output(OutputWait::new(at(99), b"x")?, &CancelScope::new()) else {
        return Err("a cursor past the head was armed".into());
    };
    assert_eq!(ahead.kind(), "invalid_params");
    let Err(empty) = OutputWait::new(at(0), b"") else {
        return Err("an empty pattern was accepted".into());
    };
    assert_eq!(empty.kind(), "invalid_pattern");
    Ok(())
}

/// Exit without the marker ends the wait as `exited`; cancellation ends it as `cancelled`.
///
/// # Errors
///
/// Fails when the hub refuses a step.
///
/// # Panics
///
/// When an asserted outcome does not hold.
pub fn exit_and_cancel_end_output_waits_distinctly() -> TestResult {
    let (waits, _screen) = hub(4096)?;
    let scope = CancelScope::new();
    let cancelled = waits.output(OutputWait::new(waits.cursor()?, b"never")?, &scope)?;
    let exited = waits.output(
        OutputWait::new(waits.cursor()?, b"never")?,
        &CancelScope::new(),
    )?;
    scope.cancel();
    assert_eq!(
        cancelled.wait(bound()?)?,
        OutputOutcome::Cancelled { cursor: at(0) }
    );
    print(&waits, b"bye")?;
    exit(&waits, 3)?;
    assert!(matches!(
        exited.wait(bound()?)?,
        OutputOutcome::Exited { exit, cursor } if exit.code == Some(3) && cursor == at(0)
    ));
    let after = waits.output(OutputWait::new(at(0), b"bye")?, &scope)?;
    assert_eq!(
        after.wait(Instant::now())?,
        OutputOutcome::Cancelled { cursor: at(0) },
        "a wait armed under a cancelled scope is never armed"
    );
    Ok(())
}

/// Outcomes carry offsets as decimal strings and round-trip at `u64::MAX`.
///
/// # Errors
///
/// Fails when serialization fails.
///
/// # Panics
///
/// When an asserted outcome does not hold.
pub fn outcomes_round_trip_decimal_offsets() -> TestResult {
    let outcome = OutputOutcome::Matched {
        at: at(u64::MAX),
        cursor: at(u64::MAX),
    };
    let text = serde_json::to_string(&outcome)?;
    assert_eq!(
        text,
        r#"{"outcome":"matched","at":"18446744073709551615","cursor":"18446744073709551615"}"#
    );
    assert_eq!(serde_json::from_str::<OutputOutcome>(&text)?, outcome);
    let numeric = r#"{"outcome":"timeout","cursor":3}"#;
    assert!(serde_json::from_str::<OutputOutcome>(numeric).is_err());
    Ok(())
}

proptest! {
    /// Every split of the output and every cursor at or before the marker finds it exactly once.
    #[test]
    fn marker_is_found_under_every_split(
        prefix in proptest::collection::vec(b'a'..=b'z', 0..24),
        split in 0usize..40,
        cursor in 0usize..24,
    ) {
        let (waits, _screen) = hub(4096).map_err(|error| TestCaseError::fail(error.to_string()))?;
        let mut bytes = prefix.clone();
        bytes.extend_from_slice(b"READY_MARK tail");
        let split = split.min(bytes.len());
        let cursor = cursor.min(prefix.len());
        let (head, tail) = bytes.split_at(split);
        let found = (|| -> TestResult<OutputOutcome> {
            print(&waits, head.get(..cursor.min(head.len())).ok_or("split")?)?;
            let start = waits.cursor()?;
            let ticket = waits.output(OutputWait::new(start, b"READY_MARK")?, &CancelScope::new())?;
            print(&waits, head.get(cursor.min(head.len())..).ok_or("split")?)?;
            print(&waits, tail)?;
            Ok(ticket.wait(Instant::now())?)
        })().map_err(|error| TestCaseError::fail(error.to_string()))?;
        let offset = u64::try_from(prefix.len()).map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(found, OutputOutcome::Matched { at: at(offset), cursor: at(offset.saturating_add(10)) });
    }
}
