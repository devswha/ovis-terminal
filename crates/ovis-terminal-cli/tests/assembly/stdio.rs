//! A program the server launches holds none of the server's standard streams: those are
//! the pipes the server's parent reads (its ready line, announcements and report), and a
//! program that outlives the server (programs survive restarts and crashes by design)
//! would keep them open, so the parent would never see end of file.

use std::collections::BTreeSet;

use serde_json::Value;

use super::restart::pid;
use super::support::{Served, TestResult};

/// The open-file targets of `pid`'s descriptors, such as `pipe:[123]`.
fn targets(pid: u64) -> TestResult<BTreeSet<String>> {
    let mut found = BTreeSet::new();
    for entry in std::fs::read_dir(format!("/proc/{pid}/fd"))? {
        if let Ok(target) = std::fs::read_link(entry?.path()) {
            found.insert(target.to_string_lossy().into_owned());
        }
    }
    Ok(found)
}

#[test]
fn a_launched_program_holds_none_of_the_servers_standard_streams() -> TestResult {
    let served = Served::start()?;
    served.run_shell("shell")?;
    let server_pid = u64::from(served.server_pid());
    let streams: BTreeSet<String> = (0..3)
        .filter_map(|fd| std::fs::read_link(format!("/proc/{server_pid}/fd/{fd}")).ok())
        .map(|target| target.to_string_lossy().into_owned())
        .filter(|target| target.starts_with("pipe:"))
        .collect();
    assert_eq!(
        streams.len(),
        3,
        "the server's stdin, stdout and stderr are pipes"
    );
    let launched = pid(&served)?;
    let program = launched
        .get(0)
        .and_then(Value::as_u64)
        .or_else(|| {
            launched
                .get(0)
                .and_then(Value::as_str)
                .and_then(|text| text.parse().ok())
        })
        .ok_or_else(|| launched.to_string())?;
    let held: Vec<String> = targets(program)?.intersection(&streams).cloned().collect();
    assert!(
        held.is_empty(),
        "program {program} holds the server's streams {held:?}"
    );
    served.stop()?;
    Ok(())
}
