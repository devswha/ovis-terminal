use super::{Membership, ProcessId, read_entry};
use crate::process::read_start_token;

fn own() -> ProcessId {
    ProcessId::new(std::process::id())
}

#[test]
fn a_live_record_carries_the_same_start_token_as_the_token_reader() {
    let entry = read_entry(own()).expect("own record");

    assert!(entry.running);
    assert_eq!(entry.pid, own());
    assert_eq!(Some(entry.start_token), read_start_token(own()).ok());
}

#[cfg(unix)]
#[test]
fn table_listing_argv_and_cwd_describe_this_process() {
    let pids = super::list_pids().expect("process table");
    let argv = super::read_argv(own()).expect("own argv");
    let cwd = super::read_cwd(own()).expect("own cwd");

    assert!(pids.contains(&own()));
    assert_eq!(argv, std::env::args().collect::<Vec<_>>());
    assert_eq!(
        cwd.canonicalize().ok(),
        std::env::current_dir()
            .and_then(|dir| dir.canonicalize())
            .ok()
    );
}

#[cfg(unix)]
#[test]
fn a_reaped_pid_is_gone_for_every_read() {
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", "exit 0"])
        .spawn()
        .expect("spawn child");
    let pid = ProcessId::new(child.id());
    assert!(child.wait().expect("child exit").success());

    let kinds = [
        read_entry(pid).err().map(|error| error.kind()),
        super::read_argv(pid).err().map(|error| error.kind()),
        super::read_cwd(pid).err().map(|error| error.kind()),
    ];

    assert_eq!(kinds, [Some("process_gone"); 3]);
}

#[cfg(unix)]
#[test]
fn an_exited_unreaped_child_keeps_its_token_but_is_not_running() {
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", "read gate; exit 0"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("spawn child");
    let pid = ProcessId::new(child.id());
    let started = read_start_token(pid).ok();
    drop(child.stdin.take());
    let raw = libc::id_t::from(pid.get());
    let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
    // SAFETY: waits for this test's own child to exit without reaping it (WNOWAIT);
    // `info` is a live, writable siginfo_t.
    let waited = unsafe {
        libc::waitid(
            libc::P_PID,
            raw,
            info.as_mut_ptr(),
            libc::WEXITED | libc::WNOWAIT,
        )
    };
    assert_eq!(waited, 0, "waitid: {}", std::io::Error::last_os_error());
    let exited = read_entry(pid);
    assert!(child.wait().expect("reap").success());

    match exited {
        Ok(entry) => {
            assert!(!entry.running);
            assert_eq!(Some(entry.start_token), started);
        }
        Err(error) => assert_eq!(error.kind(), "process_gone"),
    }
}

#[cfg(windows)]
#[test]
fn windows_reports_membership_and_listing_as_unsupported() {
    let entry = read_entry(own()).expect("own record");

    assert_eq!(entry.terminal, Membership::Unsupported);
    assert_eq!(
        super::list_pids().err().map(|error| error.kind()),
        Some("unsupported_platform")
    );
}

#[cfg(unix)]
#[test]
fn unix_membership_is_never_reported_as_unsupported() {
    let entry = read_entry(own()).expect("own record");

    assert_ne!(entry.terminal, Membership::Unsupported);
}
