//! Ordered input admission, write-boundary fencing and staged receipts (plan todo 25)
//! against the real authority, lease table, VT modes and, on Unix, a real holder PTY.

use std::error::Error;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

#[path = "../src/input/sequencer/qa/admission.rs"]
mod admission;
#[path = "../src/input/sequencer/qa/authority.rs"]
mod authority;
#[cfg(unix)]
#[path = "../src/input/sequencer/qa/blocked.rs"]
mod blocked;
#[path = "../src/input/sequencer/qa/fencing.rs"]
mod fencing;
#[path = "../src/input/sequencer/qa/fixture.rs"]
pub mod fixture;
#[path = "../src/input/sequencer/qa/leases.rs"]
mod leases;
#[path = "../src/input/sequencer/qa/model.rs"]
mod model;
#[cfg(unix)]
#[path = "../src/input/sequencer/qa/ordering.rs"]
pub mod ordering;
#[cfg(unix)]
#[path = "../src/input/sequencer/qa/pty.rs"]
pub mod pty;
#[path = "../src/input/sequencer/qa/recovery.rs"]
mod recovery;
#[cfg(unix)]
#[path = "../src/input/sequencer/qa/revocation.rs"]
pub mod revocation;
#[path = "../src/input/sequencer/qa/stages.rs"]
mod stages;
#[path = "../src/input/sequencer/qa/wire.rs"]
mod wire;

/// Happy path on a real PTY: two writers, barrier order, admission before the write.
#[cfg(unix)]
#[test]
fn happy() -> TestResult {
    ordering::two_writers_keep_submissions_contiguous_in_admission_order()
}

/// Failure path on a real PTY: takeover and credential revocation while queued.
#[cfg(unix)]
#[test]
fn failure() -> TestResult {
    revocation::revocation_while_queued_writes_no_new_bytes()
}
