//! Plan todo 62 lease owners on a real outer PTY against a real `serve`: a lease belongs to
//! the connection that acquired it, so two attach processes compete for the interact lease
//! whether they use the per-process default label or share one `--client-id`, and an
//! observer sharing its operator's label is another owner that never claims the operator's
//! geometry or input lease.
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

/// A first attach (`first` args) holds `t1`; a second attach process (`second` args) is
/// refused `lease_conflict` naming the holder, and its typing never reaches the program.
/// Returns the holder's label and the first client's pid.
fn second_attach_is_refused(
    served: &Served,
    (first, second): (&[&str], &[&str]),
) -> TestResult<(Value, u32)> {
    let before = incarnation(served)?;
    let mut first = attached(served, first, "own")?;
    let pid = first.client.id();
    let owner = served.leases()?.pointer("/interact/holder").cloned();
    let owner = owner.ok_or("no interact holder")?;
    let mut second = Outer::attach(served, second)?;
    second.type_bytes(b"echo intruder-$((3+4))\r")?;
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
    // The refused process has exited before the fence is typed, and the program reads its
    // input in order, so typing that reached it would be drawn before the fence's answer.
    first.type_bytes(b"echo fence-$((5+5))\r")?;
    assert!(
        first.shows("fence-10", WAIT),
        "the holder lost its terminal"
    );
    assert!(
        !first.shows("intruder-7", Duration::ZERO),
        "the refused attach's typing reached the program"
    );
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
    assert_eq!(incarnation(served)?, before);
    Ok((owner, pid))
}

#[test]
fn two_attach_processes_with_the_default_label_compete_and_the_second_is_refused() -> TestResult {
    let served = Served::start()?;
    let (owner, pid) = second_attach_is_refused(
        &served,
        (
            &["terminal", "attach", "--target", "t1"],
            &["--json", "terminal", "attach", "--target", "t1"],
        ),
    )?;
    assert_eq!(
        owner,
        json!(format!("cli-{pid}")),
        "the default label is per process"
    );
    served.stop()
}

#[test]
fn two_attach_processes_sharing_one_client_id_compete_and_the_second_is_refused() -> TestResult {
    let served = Served::start()?;
    let (owner, _) = second_attach_is_refused(
        &served,
        (
            &[
                "terminal",
                "attach",
                "--target",
                "t1",
                "--client-id",
                "shared",
            ],
            &[
                "--json",
                "terminal",
                "attach",
                "--target",
                "t1",
                "--client-id",
                "shared",
            ],
        ),
    )?;
    assert_eq!(owner, json!("shared"));
    served.stop()
}

#[test]
fn an_observer_sharing_its_operators_label_is_another_owner_and_releases_only_its_own_lease()
-> TestResult {
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
    let before = served.leases()?;
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
    observer.type_bytes(b"\x1c")?;
    let (code, record, restored) = observer.exited()?;
    assert_eq!(code, Some(0), "{record}");
    assert!(restored);
    // The observer's connection is its own owner: detaching releases exactly its observe
    // lease, while the operator's interact and geometry leases stay as they were.
    assert_eq!(
        record.pointer("/detached/leases_released"),
        Some(&json!(true)),
        "{record}"
    );
    assert_eq!(record.pointer("/detached/leases_held"), Some(&json!([])));
    let after = served.leases()?;
    assert_eq!(after.get("interact"), before.get("interact"), "{after}");
    assert_eq!(after.get("geometry_owner"), before.get("geometry_owner"));
    assert_eq!(after.get("observers"), Some(&json!([])), "{after}");
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
