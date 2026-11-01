//! How the binary under test is launched and how its announcements are awaited.

use std::process::{ChildStdout, Command};
use std::time::Duration;

use super::{BIN, TestResult};

/// The binary under test. With `OVIS_TEST_PIN_CORES` set (a `taskset -c` list), it runs
/// pinned to those cores, so a loaded-core run loads only the server and CLI under test
/// while the harness keeps its own cores.
pub(super) fn product() -> Command {
    std::env::var("OVIS_TEST_PIN_CORES").map_or_else(
        |_| Command::new(BIN),
        |cores| {
            let mut pinned = Command::new("/usr/bin/taskset");
            pinned.args(["-c", &cores, BIN]);
            pinned
        },
    )
}

/// Hang guard on one server announcement; a missing one fails with the live server pid.
pub(super) const ANNOUNCE_GUARD: Duration = Duration::from_secs(60);

/// Whether `stdout` has bytes (or end of file) to read within `guard`.
pub(super) fn readable(stdout: &ChildStdout, guard: Duration) -> TestResult<bool> {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let mut fds = [PollFd::new(stdout, PollFlags::IN)];
    let timeout = Timespec {
        tv_sec: i64::try_from(guard.as_secs())?,
        tv_nsec: 0,
    };
    Ok(poll(&mut fds, Some(&timeout))? > 0)
}
