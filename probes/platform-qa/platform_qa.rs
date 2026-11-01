//! First-class platform QA: the Windows smoke runner, CI verification and stress lanes.
//!
//! `windows-smoke` runs each named check's native tests one at a time under a
//! deadline and writes a report; `verify-ci` finds the CI run for a commit, requires
//! the job green and every named check passing in its downloaded report.

mod assemble;
mod bounded;
mod error;
mod evidence;
mod libtest;
mod smoke;
mod stress;

pub use assemble::sha256_hex;
pub use bounded::{Bounded, run};
pub use error::QaError;
pub use evidence::append;
pub use libtest::{NotProven, proven};
pub use smoke::{
    Check, CheckReport, MANIFEST_FORMAT, NativeTest, REPORT_FORMAT, SmokeManifest, SmokeReport,
    TestRun, Verdict,
};
pub use stress::{LANE_FORMAT, Setup, StressLane, Variant, lanes};
