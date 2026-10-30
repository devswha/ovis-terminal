//! Throwaway harness (not a test, not part of the workspace). It compiles the
//! REAL helper source of probes/platform/tests/unix_failure.rs (extracted
//! verbatim by run.sh into helper_extract.rs) and loops: spawn a thread named
//! like the probe's report thread, join it, then check for leftovers.
//!   harness single N   one `thread_names` read right after join (the old check)
//!   harness bounded N  `await_threads_at_most` with TEARDOWN_DEADLINE (the fix)
use std::time::{Duration, Instant};
use std::{io, thread};

include!("helper_extract.rs");

const NAME: &str = "probe-stall-report";

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap();
    let iterations: u32 = args.next().unwrap().parse().unwrap();
    let mut leftover = 0u32;
    for _ in 0..iterations {
        thread::Builder::new().name(NAME.into()).spawn(|| ()).unwrap().join().unwrap();
        let clean = match mode.as_str() {
            "single" => thread_names("probe-stall").unwrap().is_empty(),
            "bounded" => await_threads_at_most("probe-stall", 0, TEARDOWN_DEADLINE) == Teardown::Gone,
            other => panic!("unknown mode {}", other),
        };
        leftover += u32::from(!clean);
    }
    println!("mode={mode} iterations={iterations} leftover={leftover}");
}
