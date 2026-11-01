//! Behaviour of the harness's own logic: the schedule, the verdict, the fixture's pass
//! rule, the tally and the wire's decimal counters.

mod judging;

use std::time::Duration;

use clap::Parser as _;
use ovis_terminal_protocol::schema::scalar::EventSeq;

use super::cli::Cli;
use super::plan::{Kind, schedule};
use super::wire::Line;

const fn secs(value: u64) -> Duration {
    Duration::from_secs(value)
}

#[test]
fn every_requested_disruption_lands_inside_the_window_spaced_apart() {
    let counts = [
        (Kind::CoreCrash, 20),
        (Kind::PlannedRestart, 6),
        (Kind::RemoteInterruption, 12),
        (Kind::StalledViewer, 6),
    ];
    let list = schedule(&counts, (secs(240), secs(259_140))).unwrap();
    assert_eq!(list.len(), 44);
    for (kind, count) in counts {
        assert_eq!(list.iter().filter(|item| item.kind == kind).count(), count);
    }
    assert!(
        list.iter()
            .all(|item| item.at >= secs(240) && item.at < secs(259_140))
    );
    for (earlier, later) in list.iter().zip(list.iter().skip(1)) {
        assert!(
            later.at >= earlier.at + secs(2),
            "{earlier:?} then {later:?}"
        );
    }
}

#[test]
fn the_thirty_minute_run_crashes_twice_after_the_baseline_window() {
    let counts = [
        (Kind::CoreCrash, 2),
        (Kind::PlannedRestart, 1),
        (Kind::RemoteInterruption, 2),
        (Kind::StalledViewer, 1),
    ];
    let list = schedule(&counts, (secs(240), secs(1740))).unwrap();
    let crashes: Vec<Duration> = list
        .iter()
        .filter(|item| item.kind == Kind::CoreCrash)
        .map(|item| item.at)
        .collect();
    assert_eq!(crashes, vec![secs(615), secs(1365)]);
}

#[test]
fn disruptions_that_cannot_fit_are_refused_instead_of_squeezed_out() {
    assert_eq!(
        schedule(&[(Kind::CoreCrash, 10)], (secs(10), secs(20))),
        None
    );
    assert_eq!(
        schedule(&[(Kind::CoreCrash, 1)], (secs(20), secs(10))),
        None
    );
}

#[test]
fn sequences_are_decimal_strings_and_numbers_are_rejected() {
    let line = Line::Gap {
        from: EventSeq::new(u64::MAX - 1),
        to: EventSeq::new(u64::MAX),
    };
    let text = serde_json::to_string(&line).unwrap();
    assert_eq!(
        text,
        format!(
            r#"{{"gap":{{"from":"{}","to":"{}"}}}}"#,
            u64::MAX - 1,
            u64::MAX
        )
    );
    assert_eq!(serde_json::from_str::<Line>(&text).unwrap(), line);
    assert!(serde_json::from_str::<Line>(r#"{"gap":{"from":1,"to":"2"}}"#).is_err());
    assert!(serde_json::from_str::<Line>(r#"{"gap":{"from":"01","to":"2"}}"#).is_err());
}

#[test]
fn a_relative_binary_is_pinned_to_its_absolute_path_before_any_process_starts() {
    // nextest runs this crate's tests in its own directory, where Cargo.toml exists.
    let cli = Cli::try_parse_from([
        "soak",
        "--binary",
        "Cargo.toml",
        "--isolated",
        "--self-test",
    ])
    .unwrap();
    let (_, binary) = cli.plan().unwrap();
    assert!(binary.is_absolute(), "{}", binary.display());
    assert_eq!(binary, std::fs::canonicalize("Cargo.toml").unwrap());
    let missing = Cli::try_parse_from([
        "soak",
        "--binary",
        "no-such-binary",
        "--isolated",
        "--self-test",
    ])
    .unwrap();
    assert_eq!(
        missing.plan().err().map(|error| error.kind()),
        Some("binary_unavailable")
    );
}
