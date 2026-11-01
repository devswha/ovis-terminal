//! Real in-process runs: terminal actors fed by paced in-process programs, real client
//! sockets, no child process.

use std::time::Duration;

use super::super::drive;
use super::super::host::Binary;
use super::super::plan::{Fixture, Plan, Profile, Shape, Source};

const IDEAL: Profile = Profile::Ideal {
    admission_max_ms: 100,
    event_max_ms: 100,
};

fn in_process(name: &str, fixture: Option<Fixture>) -> Plan {
    let root = std::env::var_os("OVIS_FIXTURE_ROOT")
        .map_or_else(std::env::temp_dir, Into::into)
        .join(format!("terminal50-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    Plan {
        shape: Shape {
            terminals: 2,
            clients: 2,
            inflight: 4,
            bytes_per_second: 64 * 1024,
        },
        profile: IDEAL,
        samples: 40,
        warmup: 0,
        viewers: 2,
        rss_limit_bytes: u64::MAX,
        disk_limit_bytes: u64::MAX,
        workers: 2,
        max_run: Duration::from_secs(60),
        drain: Duration::from_secs(1),
        fixture,
        source: Source::InProcess,
        root,
    }
}

fn binary() -> Binary {
    Binary {
        path: "in-process".to_owned(),
        sha256: String::new(),
        bytes: 0,
        version: String::new(),
    }
}

#[test]
fn a_delayed_control_writer_breaks_the_event_budget_with_every_sample_kept() {
    let plan = in_process(
        "delayed",
        Some(Fixture::ControlWriterDelay { delay_ms: 150 }),
    );
    let (report, raw) = drive::run(&plan, binary()).unwrap();
    std::fs::remove_dir_all(&plan.root).unwrap();
    assert!(!report.verdict.passed);
    let event = report
        .verdict
        .violations
        .iter()
        .find(|violation| violation.check == "event_max_ms")
        .unwrap();
    assert!(
        event
            .observed
            .get("ms")
            .and_then(serde_json::Value::as_f64)
            .unwrap()
            >= 150.0,
        "{event:?}"
    );
    assert!(
        raw.event
            .iter()
            .chain(&raw.event_warmup)
            .all(|us| *us >= 150_000)
    );
    assert_eq!(report.budget, IDEAL, "the budget is never widened");
}

#[test]
fn every_committed_event_is_delivered_lost_or_undelivered_for_each_client() {
    let plan = in_process("accounting", None);
    let (report, raw) = drive::run(&plan, binary()).unwrap();
    std::fs::remove_dir_all(&plan.root).unwrap();
    assert!(report.events_committed > 0);
    assert!(raw.admission.len() >= 40);
    assert_eq!(report.clients.len(), 3);
    assert_eq!(
        report.clients.iter().filter(|client| client.reads).count(),
        2
    );
    for client in &report.clients {
        let delivered = u64::try_from(client.delivered).unwrap();
        let accounted = delivered + client.lost + client.undelivered;
        if client.lost == 0 {
            assert_eq!(accounted, report.events_committed, "{client:?}");
        } else {
            assert!(accounted >= report.events_committed, "{client:?}");
        }
    }
    assert_eq!(
        report.viewers.iter().filter(|viewer| !viewer.reads).count(),
        2
    );
    assert!(
        report
            .viewers
            .iter()
            .filter(|viewer| viewer.reads)
            .all(|viewer| viewer.bytes > 0)
    );
}
