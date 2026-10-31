//! Actual child ownership is retained through the PTY EOF barrier.

use std::ffi::OsString;
use std::sync::{Arc, mpsc};
use std::thread;

use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};

use super::Holder;
use super::ending::ExitEvidence;
use super::threads::{Waiter, wait_for_exit};
use crate::unix::ProcessEnvironment;
use crate::unix::drain::DrainSignal;
use crate::unix::launch::{Grid, LaunchSpec, launch};
use crate::unix::pty_io::{ReadOutcome, read_chunk};
use crate::unix::test_fixtures::{WAIT, config};

#[test]
fn child_is_unreaped_at_the_real_pty_hangup_barrier() {
    // Given: a real PTY child and the production waiter, with the pump barrier held.
    let spec = LaunchSpec::new(
        ["/bin/sh", "-c", "printf final; exit 7"]
            .map(OsString::from)
            .to_vec(),
        ProcessEnvironment::new(std::env::temp_dir(), Vec::new()),
        Grid::new(80, 24).expect("grid"),
    );
    let mut process = launch(&spec).expect("real PTY child");
    let pid = Pid::from_child(&process.child);
    let drain = Arc::new(DrainSignal::default());
    let holder = Holder::new(config());
    let (sender, receiver) = mpsc::sync_channel(1);
    let waiter = Waiter {
        shared: Arc::clone(&holder.shared),
        terminal_id: crate::protocol::TerminalId::new("t-reap".into()),
        child: process.child,
        slave: process.slave,
        probe: process.probe,
        drain: Arc::clone(&drain),
    };
    let worker = thread::spawn(move || wait_for_exit(waiter, &sender));
    let mut bytes = Vec::new();
    let mut buffer = [0; 64];

    // When: the master actually reaches EOF, before releasing the pump barrier.
    while let ReadOutcome::Data(count) =
        read_chunk(&mut process.reader, &mut buffer).expect("PTY read")
    {
        bytes.extend_from_slice(buffer.get(..count).expect("read bounds"));
        drain.advance();
    }
    let unreaped = waitid(
        WaitId::Pid(pid),
        WaitIdOptions::EXITED | WaitIdOptions::NOWAIT | WaitIdOptions::NOHANG,
    );
    drain.finish();
    let report = receiver.recv_timeout(WAIT).expect("bounded waiter report");
    worker.join().expect("waiter joined");

    // Then: status was still waitable at EOF, and only the barrier allowed reaping.
    assert!(unreaped.expect("child must not have been reaped").is_some());
    assert_eq!(bytes, b"final");
    assert!(matches!(
        report.evidence,
        ExitEvidence::Status {
            code: Some(7),
            signal: None
        }
    ));
    assert_eq!(
        waitid(WaitId::Pid(pid), WaitIdOptions::EXITED).expect_err("now reaped"),
        rustix::io::Errno::CHILD
    );
}
