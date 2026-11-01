//! Plan todo 62 lease ownership on a real outer PTY against a real `serve`: explicit
//! takeover bound to an epoch, held attaches that name their owner and retry boundary,
//! and observers that never take geometry or input and outlive the operator.
#![cfg(unix)]

#[path = "attach/support.rs"]
pub mod support;

#[path = "interactive_attach/helpers.rs"]
pub mod helpers;

use std::time::Duration;

use helpers::{attached, incarnation};
use rustix::termios::{self, Winsize};
use serde_json::{Value, json};
use support::{Outer, Served, TestResult, WAIT};

/// A competing attach and a stale takeover are both refused, naming the live holder.
fn refused_without_eviction(served: &Served, epoch: &Value) -> TestResult {
    let held = Outer::attach(
        served,
        &[
            "--json",
            "terminal",
            "attach",
            "--target",
            "t1",
            "--client-id",
            "rival",
        ],
    )?;
    let (code, record, restored) = held.exited()?;
    assert_eq!(code, Some(11), "{record}");
    assert_eq!(
        record.pointer("/error/kind"),
        Some(&json!("lease_conflict"))
    );
    assert_eq!(record.pointer("/error/holder/holder"), Some(&json!("cli")));
    assert_eq!(
        record.pointer("/error/retry/on"),
        Some(&json!("lease_released"))
    );
    assert_eq!(
        record.pointer("/error/retry/event"),
        Some(&json!("lease.revoked"))
    );
    assert_eq!(record.pointer("/error/retry/lease/epoch"), Some(epoch));
    assert!(restored);
    let stale = Outer::attach(
        served,
        &[
            "--json",
            "terminal",
            "attach",
            "--target",
            "t1",
            "--client-id",
            "bridge",
            "--takeover",
            "999",
        ],
    )?;
    let (code, record, restored) = stale.exited()?;
    assert_eq!(code, Some(11), "{record}");
    assert_eq!(
        record.pointer("/error/kind"),
        Some(&json!("lease_epoch_mismatch"))
    );
    assert_eq!(record.pointer("/error/expected_epoch"), Some(&json!("999")));
    assert_eq!(record.pointer("/error/current/epoch"), Some(epoch));
    assert_eq!(record.pointer("/error/current/holder"), Some(&json!("cli")));
    assert!(restored);
    assert_eq!(
        served.leases()?.pointer("/interact/holder"),
        Some(&json!("cli")),
        "no eviction"
    );
    Ok(())
}

#[test]
fn a_stale_takeover_names_the_current_epoch_and_a_current_one_displaces_the_holder() -> TestResult {
    let served = Served::start()?;
    let before = incarnation(&served)?;
    let first = attached(&served, &["terminal", "attach", "--target", "t1"], "first")?;
    let epoch = served
        .leases()?
        .pointer("/interact/epoch")
        .cloned()
        .ok_or("epoch")?;
    refused_without_eviction(&served, &epoch)?;
    let epoch = epoch.as_str().ok_or("decimal epoch")?;
    let mut bridge = Outer::attach(
        &served,
        &[
            "terminal",
            "attach",
            "--target",
            "t1",
            "--client-id",
            "bridge",
            "--takeover",
            epoch,
        ],
    )?;
    let (code, record, restored) = first.exited()?;
    assert_eq!(code, Some(0), "{record}");
    assert_eq!(
        record.pointer("/closed/close/reason"),
        Some(&json!("taken_over"))
    );
    assert_eq!(
        record.pointer("/closed/close/by/holder"),
        Some(&json!("bridge"))
    );
    assert!(restored);
    bridge.type_bytes(b"echo bridge-$((2+2))\r")?;
    assert!(bridge.shows("bridge-4", WAIT));
    bridge.type_bytes(b"\x1c")?;
    let (code, record, restored) = bridge.exited()?;
    assert_eq!(code, Some(0), "{record}");
    assert_eq!(
        record.pointer("/detached/displaced/holder"),
        Some(&json!("cli"))
    );
    assert_eq!(
        record.pointer("/detached/leases_released"),
        Some(&json!(true))
    );
    assert!(restored);
    assert_eq!(
        incarnation(&served)?,
        before,
        "takeover and detach never touch the program"
    );
    served.stop()
}

#[test]
fn an_observer_on_a_terminal_never_resizes_and_stays_live_after_the_operator_detaches() -> TestResult
{
    let served = Served::start()?;
    let mut operator = attached(&served, &["terminal", "attach", "--target", "t1"], "op")?;
    let mut observer = Outer::attach(
        &served,
        &[
            "terminal",
            "attach",
            "--target",
            "t1",
            "--observe",
            "--client-id",
            "phone",
        ],
    )?;
    operator.type_bytes(b"echo seen-$((6+1))\r")?;
    assert!(
        observer.shows("seen-7", WAIT),
        "the observer never painted the program"
    );
    let geometry = served.leases()?.pointer("/geometry_owner").cloned();
    termios::tcsetwinsize(
        &observer.master,
        Winsize {
            ws_row: 11,
            ws_col: 33,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )?;
    observer.type_bytes(b"echo rogue-$((1+1))\r")?;
    operator.type_bytes(b"stty size\r")?;
    assert!(
        operator.shows("30 100", WAIT),
        "the observer's grid reached the program"
    );
    assert!(
        !operator.shows("rogue-2", Duration::ZERO),
        "observer typing reached the program"
    );
    assert_eq!(
        served.leases()?.pointer("/geometry_owner").cloned(),
        geometry
    );
    operator.type_bytes(b"\x1c")?;
    assert_eq!(operator.exited()?.0, Some(0));
    let leases = served.leases()?;
    assert_eq!(leases.pointer("/interact"), Some(&Value::Null));
    assert_eq!(
        leases.pointer("/observers/0/role"),
        Some(&json!("observe")),
        "{leases}"
    );
    observer.type_bytes(b"\x1c")?;
    let (code, record, restored) = observer.exited()?;
    assert_eq!(code, Some(0), "{record}");
    assert_eq!(
        record.pointer("/detached/reason"),
        Some(&json!("quit_signal"))
    );
    assert_eq!(
        record.pointer("/detached/leases_released"),
        Some(&json!(true))
    );
    assert!(restored);
    served.stop()
}
