//! `ovis-terminal terminal attach` on a real outer PTY against a real `serve`: typed input
//! reaches the program, resizes follow, Ctrl-\ detaches, and the outer terminal's
//! settings are back exactly as they were after every way an attach can end.
#![cfg(unix)]

#[path = "attach/support.rs"]
pub mod support;

use std::io::Write as _;
use std::process::Stdio;
use std::time::Duration;

use rustix::termios::{self, Winsize};
use serde_json::{Value, json};
use support::{Outer, Served, TestResult, WAIT};

#[test]
fn typed_input_reaches_the_program_resize_follows_and_ctrl_backslash_restores_the_terminal()
-> TestResult {
    let served = Served::start()?;
    let mut outer = Outer::attach(&served, &["terminal", "attach", "--target", "t1"])?;
    outer.type_bytes(b"echo mark-$((30+1))\r")?;
    assert!(
        outer.shows("mark-31", WAIT),
        "the program never echoed the marker"
    );
    assert_eq!(
        served.leases()?.pointer("/interact/holder"),
        Some(&json!("cli"))
    );
    termios::tcsetwinsize(
        &outer.master,
        Winsize {
            ws_row: 40,
            ws_col: 120,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )?;
    let resized = (0..20).any(|_| {
        outer.type_bytes(b"stty size\r").is_ok() && outer.shows("40 120", Duration::from_secs(1))
    });
    assert!(resized, "the inner PTY never reported the new size");
    outer.type_bytes(b"\x1c")?;
    let (code, record, restored) = outer.exited()?;
    assert_eq!(code, Some(0), "{record}");
    assert_eq!(
        record.pointer("/detached/reason"),
        Some(&json!("detach_key"))
    );
    assert_eq!(
        record.pointer("/detached/stream_closed"),
        Some(&json!(true))
    );
    assert!(restored, "the outer terminal settings changed");
    assert_eq!(
        served.leases()?,
        json!({"interact": null, "geometry_owner": null, "observers": []})
    );
    served.stop()
}

#[test]
fn ctrl_c_goes_to_the_program_and_sigint_to_the_client_detaches_cleanly() -> TestResult {
    let served = Served::start()?;
    let mut outer = Outer::attach(&served, &["terminal", "attach", "--target", "t1"])?;
    outer.type_bytes(b"echo up-$((1+1))\r")?;
    assert!(outer.shows("up-2", WAIT));
    outer.type_bytes(b"sleep 100\r")?;
    assert!(outer.shows("sleep 100", WAIT));
    outer.type_bytes(b"\x03")?;
    assert!(
        outer.shows("^C", WAIT),
        "the program's terminal never saw Ctrl-C"
    );
    outer.type_bytes(b"echo after-$((5+5))\r")?;
    assert!(
        outer.shows("after-10", WAIT),
        "Ctrl-C did not reach the program"
    );
    let pid = rustix::process::Pid::from_raw(i32::try_from(outer.client.id())?).ok_or("pid")?;
    assert!(pid.as_raw_nonzero().get() > 1);
    rustix::process::kill_process(pid, rustix::process::Signal::INT)?;
    let (code, record, restored) = outer.exited()?;
    assert_eq!(code, Some(0), "{record}");
    assert_eq!(
        record.pointer("/detached/reason"),
        Some(&json!("interrupted"))
    );
    assert!(restored);
    served.stop()
}

#[test]
fn a_second_interact_attach_is_held_with_the_owner_and_never_touches_its_terminal() -> TestResult {
    let served = Served::start()?;
    let mut first = Outer::attach(&served, &["terminal", "attach", "--target", "t1"])?;
    first.type_bytes(b"echo one-$((0+1))\r")?;
    assert!(first.shows("one-1", WAIT));
    let second = Outer::attach(
        &served,
        &[
            "--json",
            "terminal",
            "attach",
            "--target",
            "t1",
            "--client-id",
            "second",
        ],
    )?;
    let (code, record, restored) = second.exited()?;
    assert_eq!(code, Some(11), "{record}");
    assert_eq!(record.pointer("/error/kind"), Some(&json!("lease_held")));
    assert_eq!(record.pointer("/error/holder/holder"), Some(&json!("cli")));
    assert!(restored, "a held attach changed the outer terminal");
    let stale = Outer::attach(&served, &["--json", "terminal", "attach", "--target", "t9"])?;
    let (code, record, restored) = stale.exited()?;
    assert_eq!(code, Some(3), "{record}");
    assert_eq!(record.pointer("/error/kind"), Some(&json!("not_found")));
    assert!(restored);
    first.type_bytes(b"\x1c")?;
    assert_eq!(first.exited()?.0, Some(0));
    served.stop()
}

#[test]
fn observing_needs_no_tty_and_never_forwards_input() -> TestResult {
    let served = Served::start()?;
    let mut interact = Outer::attach(&served, &["terminal", "attach", "--target", "t1"])?;
    interact.type_bytes(b"echo ready-$((2+3))\r")?;
    assert!(interact.shows("ready-5", WAIT));
    let mut observer = served
        .command(&["terminal", "attach", "--target", "t1", "--observe"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    observer
        .stdin
        .take()
        .ok_or("stdin")?
        .write_all(b"echo rogue-$((1+1))\r")?;
    interact.type_bytes(b"echo after-$((4+4))\r")?;
    assert!(interact.shows("after-8", WAIT));
    assert!(
        !interact.shows("rogue-2", Duration::from_millis(300)),
        "observer input reached the program"
    );
    let pid = rustix::process::Pid::from_raw(i32::try_from(observer.id())?).ok_or("pid")?;
    rustix::process::kill_process(pid, rustix::process::Signal::TERM)?;
    let output = observer.wait_with_output()?;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("after-8"));
    let record: Value = serde_json::from_slice(&output.stderr)?;
    assert_eq!(
        record.pointer("/detached/reason"),
        Some(&json!("terminated"))
    );
    let refused = served
        .command(&["--json", "terminal", "attach", "--target", "t1"])
        .stdin(Stdio::null())
        .output()?;
    assert_eq!(refused.status.code(), Some(2), "{refused:?}");
    let record: Value = serde_json::from_slice(&refused.stderr)?;
    assert_eq!(record.pointer("/error/kind"), Some(&json!("not_a_tty")));
    interact.type_bytes(b"\x1c")?;
    assert_eq!(interact.exited()?.0, Some(0));
    served.stop()
}

#[test]
fn connection_loss_restores_the_outer_terminal_and_reports_it() -> TestResult {
    let served = Served::start()?;
    let mut outer = Outer::attach(&served, &["--json", "terminal", "attach", "--target", "t1"])?;
    outer.type_bytes(b"echo live-$((3+4))\r")?;
    assert!(outer.shows("live-7", WAIT));
    served.stop()?;
    let (code, record, restored) = outer.exited()?;
    assert_eq!(code, Some(4), "{record}");
    assert_eq!(
        record.pointer("/error/kind"),
        Some(&json!("connection_lost"))
    );
    assert!(restored, "connection loss left the outer terminal raw");
    Ok(())
}
