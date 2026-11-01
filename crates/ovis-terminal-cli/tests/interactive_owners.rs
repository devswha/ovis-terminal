//! Plan todo 62 lease owners on a real outer PTY against a real `serve`: the default client
//! label is per process, so two attach processes compete for the interact lease instead of
//! sharing it, and an observer that explicitly shares its operator's label never claims that
//! owner's geometry or input lease.
#![cfg(unix)]

#[path = "attach/support.rs"]
pub mod support;

#[path = "interactive_attach/helpers.rs"]
pub mod helpers;

use helpers::{attached, incarnation};
use rustix::termios::{self, Winsize};
use serde_json::{Value, json};
use support::{Outer, Served, TestResult, WAIT};

#[test]
fn two_attach_processes_with_the_default_label_compete_and_the_second_is_refused() -> TestResult {
    let served = Served::start()?;
    let before = incarnation(&served)?;
    let mut first = attached(&served, &["terminal", "attach", "--target", "t1"], "own")?;
    let owner = json!(format!("cli-{}", first.client.id()));
    let second = Outer::attach(&served, &["--json", "terminal", "attach", "--target", "t1"])?;
    let (code, record, restored) = second.exited()?;
    assert_eq!(
        code,
        Some(11),
        "the second attach shared the lease: {record}"
    );
    assert_eq!(
        record.pointer("/error/kind"),
        Some(&json!("lease_conflict"))
    );
    assert_eq!(record.pointer("/error/holder/holder"), Some(&owner));
    assert_eq!(
        record.pointer("/error/retry/event"),
        Some(&json!("lease.revoked"))
    );
    assert!(restored);
    assert_eq!(served.leases()?.pointer("/interact/holder"), Some(&owner));
    first.type_bytes(b"\x1c")?;
    let (code, record, restored) = first.exited()?;
    assert_eq!(code, Some(0), "{record}");
    assert_eq!(
        record.pointer("/detached/leases_released"),
        Some(&json!(true)),
        "{record}"
    );
    assert!(restored);
    let leases = served.leases()?;
    assert_eq!(leases.pointer("/interact"), Some(&Value::Null), "{leases}");
    assert_eq!(leases.pointer("/geometry_owner"), Some(&Value::Null));
    assert_eq!(incarnation(&served)?, before);
    served.stop()
}

#[test]
fn an_observer_sharing_its_operators_label_never_claims_the_owners_geometry_or_input() -> TestResult
{
    let served = Served::start()?;
    let desk = [
        "terminal",
        "attach",
        "--target",
        "t1",
        "--client-id",
        "desk",
    ];
    let mut operator = attached(&served, &desk, "op")?;
    let mut observer = Outer::attach(
        &served,
        &[
            "terminal",
            "attach",
            "--target",
            "t1",
            "--observe",
            "--client-id",
            "desk",
        ],
    )?;
    operator.type_bytes(b"echo seen-$((6+1))\r")?;
    assert!(observer.shows("seen-7", WAIT), "the observer never painted");
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
    operator.type_bytes(b"stty size\r")?;
    assert!(
        operator.shows("30 100", WAIT),
        "the observer's grid reached the program"
    );
    assert_eq!(
        served.leases()?.pointer("/geometry_owner").cloned(),
        geometry
    );
    observer.type_bytes(b"\x1c")?;
    let (code, record, restored) = observer.exited()?;
    assert_eq!(code, Some(0), "{record}");
    assert!(restored);
    // The owner keeps its leases while its operator stream remains; the observer may only
    // report its own observe lease as held, never the owner's geometry or input lease.
    let held = record
        .pointer("/detached/leases_held")
        .and_then(Value::as_array);
    let held = held.ok_or_else(|| format!("no leases_held in {record}"))?;
    assert!(
        held.iter()
            .all(|lease| lease.get("role") == Some(&json!("observe"))),
        "the observer claimed its owner's lease: {record}"
    );
    operator.type_bytes(b"\x1c")?;
    let (code, record, restored) = operator.exited()?;
    assert_eq!(code, Some(0), "{record}");
    assert_eq!(
        record.pointer("/detached/leases_released"),
        Some(&json!(true)),
        "{record}"
    );
    assert!(restored);
    assert_eq!(
        served.leases()?,
        json!({"interact": null, "geometry_owner": null, "observers": []})
    );
    served.stop()
}
