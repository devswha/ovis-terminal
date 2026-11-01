//! Plan todo 62 terminal restore on a real outer PTY against a real `serve`: unknown
//! sequences pass through unchanged, the program's modes are switched off on detach, and
//! interruption at every stage leaves the outer terminal and the program as they were.
#![cfg(unix)]

#[path = "attach/support.rs"]
pub mod support;

#[path = "interactive_attach/helpers.rs"]
pub mod helpers;

use std::io::Read as _;
use std::os::unix::net::UnixListener;
use std::process::{Command, Stdio};

use helpers::{attached, incarnation};
use serde_json::{Value, json};
use support::{BIN, Served, TestResult, WAIT};

fn signal(pid: u32, signal: rustix::process::Signal) -> TestResult {
    let pid = rustix::process::Pid::from_raw(i32::try_from(pid)?).ok_or("pid")?;
    assert!(pid.as_raw_nonzero().get() > 1, "never signal pid 0 or 1");
    Ok(rustix::process::kill_process(pid, signal)?)
}

#[test]
fn unknown_sequences_reach_the_program_and_its_modes_are_switched_off_on_detach() -> TestResult {
    let served = Served::start()?;
    let before = incarnation(&served)?;
    let mut outer = attached(&served, &["terminal", "attach", "--target", "t1"], "modes")?;
    outer.type_bytes(b"cat -v\r")?;
    assert!(outer.shows("cat -v", WAIT));
    outer.type_bytes(b"\x1b[99;7~\xfe\r")?;
    assert!(
        outer.shows("^[[99;7~M-~", WAIT),
        "an unknown sequence was not passed through"
    );
    outer.type_bytes(b"\x04")?;
    outer.type_bytes(
        b"printf '\\033[?1049h\\033[?1000;1006h\\033[?2004h\\033[?25l'; echo on-$((3*3))\r",
    )?;
    assert!(outer.shows("on-9", WAIT));
    outer.type_bytes(b"\x1c")?;
    assert!(
        outer.shows(
            "\x1b[?1000l\x1b[?1006l\x1b[?2004l\x1b[?1049l\x1b[?25h\x1b[0m",
            WAIT
        ),
        "the modes the program switched on were not switched off"
    );
    let (code, record, restored) = outer.exited()?;
    assert_eq!(code, Some(0), "{record}");
    assert_eq!(
        record.pointer("/detached/reason"),
        Some(&json!("detach_key"))
    );
    assert_eq!(
        record.pointer("/detached/leases_released"),
        Some(&json!(true))
    );
    assert!(restored);
    assert_eq!(
        incarnation(&served)?,
        before,
        "Ctrl-\\ detached without touching the program"
    );
    served.stop()
}

#[test]
fn repeated_signals_while_attached_detach_once_and_restore_the_terminal() -> TestResult {
    let served = Served::start()?;
    let before = incarnation(&served)?;
    let outer = attached(&served, &["terminal", "attach", "--target", "t1"], "sig")?;
    for kind in [
        rustix::process::Signal::TERM,
        rustix::process::Signal::INT,
        rustix::process::Signal::HUP,
        rustix::process::Signal::QUIT,
    ] {
        signal(outer.client.id(), kind)?;
    }
    let (code, record, restored) = outer.exited()?;
    assert_eq!(code, Some(0), "{record}");
    let reason = record.pointer("/detached/reason").and_then(Value::as_str);
    assert!(
        matches!(
            reason,
            Some("terminated" | "interrupted" | "hangup" | "quit_signal")
        ),
        "{record}"
    );
    assert_eq!(
        record.pointer("/detached/leases_released"),
        Some(&json!(true)),
        "{record}"
    );
    assert!(
        restored,
        "repeated interruption left the outer terminal changed"
    );
    assert_eq!(
        served.leases()?,
        json!({"interact": null, "geometry_owner": null, "observers": []})
    );
    assert_eq!(incarnation(&served)?, before);
    served.stop()
}

#[test]
fn a_signal_before_the_stream_is_attached_is_a_typed_interruption_at_its_stage() -> TestResult {
    let root = std::env::temp_dir().join(format!("ovt62-{}", std::process::id()));
    std::fs::create_dir_all(&root)?;
    let socket = root.join("s.sock");
    let listener = UnixListener::bind(&socket)?;
    let mut client = Command::new(BIN)
        .args([
            "--json",
            "terminal",
            "attach",
            "--target",
            "t1",
            "--observe",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("OVIS_TERMINAL_SOCKET", &socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let (mut accepted, _) = listener.accept()?;
    let mut hello = [0_u8; 1];
    accepted.read_exact(&mut hello)?;
    signal(client.id(), rustix::process::Signal::INT)?;
    let mut stderr = String::new();
    client
        .stderr
        .take()
        .ok_or("stderr")?
        .read_to_string(&mut stderr)?;
    let status = client.wait()?;
    let record: Value = serde_json::from_str(stderr.lines().last().unwrap_or("null"))?;
    assert_eq!(status.code(), Some(10), "{stderr}");
    assert_eq!(record.pointer("/error/kind"), Some(&json!("interrupted")));
    assert_eq!(record.pointer("/error/stage"), Some(&json!("connecting")));
    assert_eq!(record.pointer("/error/signal"), Some(&json!("SIGINT")));
    drop(accepted);
    std::fs::remove_dir_all(&root)?;
    Ok(())
}
