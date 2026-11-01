//! A run whose fleet came up short of the requested shape fails, whatever its latency.

use std::time::Duration;

use super::super::plan::Profile;
use super::super::verdict::{Budgets, Observed, Realized, judge};

#[test]
fn fewer_terminals_viewers_clients_or_lanes_than_requested_fail_the_verdict() {
    let requested = Realized {
        terminals: 50,
        viewers: 400,
        clients: 8,
        lanes: 16,
    };
    let fast = vec![Duration::from_millis(1); 100];
    let mut observed = Observed {
        admission: fast.clone(),
        event: fast,
        wanted_samples: 100,
        requested,
        realized: requested,
        ..Observed::default()
    };
    let budgets = Budgets {
        rss_limit_bytes: u64::MAX,
        disk_limit_bytes: u64::MAX,
    };
    let profile = Profile::Capacity {
        admission_p99_ms: 100,
        event_p99_ms: 100,
    };
    let checks = |observed: &Observed| -> Vec<&'static str> {
        judge(profile, observed, budgets)
            .into_iter()
            .map(|violation| violation.check)
            .collect()
    };
    assert_eq!(checks(&observed), Vec::<&str>::new());
    observed.realized = Realized {
        terminals: 2,
        viewers: 16,
        clients: 7,
        lanes: 15,
    };
    assert_eq!(
        checks(&observed),
        [
            "terminals_realized",
            "viewers_realized",
            "clients_realized",
            "lanes_realized"
        ]
    );
}
