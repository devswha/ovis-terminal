//! The spawn path reads the start token of a program that exits at once.

use std::ffi::OsString;

use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};

use super::config::ProcessEnvironment;
use super::launch::{Grid, LaunchSpec, launch};
use super::test_fixtures::shell;
use crate::process::read_start_token;

#[test]
fn a_program_that_exits_at_once_launches_with_the_token_it_keeps_until_reaped() {
    let request = shell("t-fast-exit", "exit 0");
    let spec = LaunchSpec::new(
        request.argv.into_iter().map(OsString::from).collect(),
        ProcessEnvironment::new(
            request.cwd,
            request
                .env
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        ),
        Grid::new(80, 24).expect("grid"),
    );

    let mut process = launch(&spec).expect("a fast-exiting program still launches");
    waitid(
        WaitId::Pid(Pid::from_child(&process.child)),
        WaitIdOptions::EXITED | WaitIdOptions::NOWAIT,
    )
    .expect("the child became waitable without being reaped");
    let unreaped = read_start_token(process.pid).ok();
    let status = process.child.wait().expect("reap");
    let reaped = read_start_token(process.pid)
        .err()
        .map(|error| error.kind());

    assert_eq!(unreaped, Some(process.start_token));
    assert!(status.success());
    assert_eq!(reaped, Some("process_gone"));
}
